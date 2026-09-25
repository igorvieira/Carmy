//! Attribute macro implementing Carmy's typed Tool trait.
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    FnArg, GenericArgument, ItemFn, PathArguments, ReturnType, Type, parse_macro_input,
    spanned::Spanned,
};

#[proc_macro_attribute]
pub fn tool(args: TokenStream, input: TokenStream) -> TokenStream {
    let function = parse_macro_input!(input as ItemFn);
    match expand(args.into(), function) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

const SIGNATURE: &str = "a Carmy tool is `async fn name([ctx: AgentContext,] [State(x): State<T>, ...] [input: Input]) -> AgentResult<Output>`";

/// Role of one function parameter, decided by its type.
enum Param {
    Context,
    State(Type),
    Input(Type),
}
fn last_segment(ty: &Type) -> Option<&syn::PathSegment> {
    match ty {
        Type::Path(path) if path.qself.is_none() => path.path.segments.last(),
        _ => None,
    }
}
fn single_type_argument(segment: &syn::PathSegment) -> Option<&Type> {
    match &segment.arguments {
        PathArguments::AngleBracketed(args) if args.args.len() == 1 => match &args.args[0] {
            GenericArgument::Type(ty) => Some(ty),
            _ => None,
        },
        _ => None,
    }
}
fn classify(arg: &FnArg) -> syn::Result<Param> {
    let FnArg::Typed(arg) = arg else {
        return Err(syn::Error::new(arg.span(), "tools cannot take `self`"));
    };
    let segment = last_segment(&arg.ty);
    Ok(match segment {
        Some(s) if s.ident == "AgentContext" => Param::Context,
        Some(s) if s.ident == "State" => match single_type_argument(s) {
            Some(ty) => Param::State(ty.clone()),
            None => {
                return Err(syn::Error::new(arg.ty.span(), "expected `State<T>`"));
            }
        },
        _ => Param::Input((*arg.ty).clone()),
    })
}

struct Attributes {
    description: String,
    effect: Option<String>,
    idempotent: bool,
    parallel_safe: bool,
    confirmation: String,
    register: bool,
}
fn attributes(args: proc_macro2::TokenStream) -> syn::Result<Attributes> {
    let mut a = Attributes {
        description: String::new(),
        effect: None,
        idempotent: false,
        parallel_safe: false,
        confirmation: "none".into(),
        register: true,
    };
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("description") {
            a.description = meta.value()?.parse::<syn::LitStr>()?.value();
        } else if meta.path.is_ident("effect") {
            a.effect = Some(meta.value()?.parse::<syn::LitStr>()?.value());
        } else if meta.path.is_ident("idempotent") {
            a.idempotent = meta.value()?.parse::<syn::LitBool>()?.value;
        } else if meta.path.is_ident("parallel_safe") {
            a.parallel_safe = meta.value()?.parse::<syn::LitBool>()?.value;
        } else if meta.path.is_ident("confirmation") {
            a.confirmation = meta.value()?.parse::<syn::LitStr>()?.value();
        } else if meta.path.is_ident("register") {
            a.register = meta.value()?.parse::<syn::LitBool>()?.value;
        } else {
            return Err(meta.error(
                "unknown Carmy tool attribute; expected description, effect, idempotent, parallel_safe, confirmation or register",
            ));
        }
        Ok(())
    });
    syn::parse::Parser::parse2(parser, args)?;
    Ok(a)
}

fn expand(args: proc_macro2::TokenStream, f: ItemFn) -> syn::Result<proc_macro2::TokenStream> {
    let attrs = attributes(args)?;
    let err = |message: &str| syn::Error::new_spanned(&f.sig, message);
    if f.sig.asyncness.is_none() {
        return Err(err(&format!("tool must be async; {SIGNATURE}")));
    }
    if !f.sig.generics.params.is_empty() || f.sig.unsafety.is_some() || f.sig.abi.is_some() {
        return Err(err(&format!(
            "tool must be a safe fn without generics; {SIGNATURE}"
        )));
    }
    let params = f
        .sig
        .inputs
        .iter()
        .map(classify)
        .collect::<syn::Result<Vec<_>>>()?;
    if params
        .iter()
        .filter(|p| matches!(p, Param::Context))
        .count()
        > 1
    {
        return Err(err("tool takes at most one AgentContext"));
    }
    let inputs: Vec<_> = params
        .iter()
        .filter_map(|p| match p {
            Param::Input(ty) => Some(ty),
            _ => None,
        })
        .collect();
    if inputs.len() > 1 {
        return Err(err(
            "tool takes at most one input; combine the fields into one `#[derive(Deserialize, JsonSchema)]` struct",
        ));
    }
    let input = match inputs.first() {
        Some(ty) => quote!(#ty),
        None => quote!(::carmy::NoInput),
    };
    let output = match &f.sig.output {
        ReturnType::Type(_, ty) => last_segment(ty)
            .filter(|s| s.ident == "AgentResult")
            .and_then(single_type_argument),
        ReturnType::Default => None,
    }
    .ok_or_else(|| err(&format!("expected `-> AgentResult<Output>`; {SIGNATURE}")))?;
    let effect = match attrs.effect.as_deref() {
        Some("none") => quote!(None),
        Some("read") => quote!(Read),
        Some("write") => quote!(Write),
        Some("external_write") => quote!(ExternalWrite),
        Some("destructive") => quote!(Destructive),
        _ => {
            return Err(err(
                "declare effect = \"none\", \"read\", \"write\", \"external_write\", or \"destructive\"",
            ));
        }
    };
    let confirmation = match attrs.confirmation.as_str() {
        "none" => quote!(None),
        "required" => quote!(Required),
        _ => return Err(err("confirmation must be \"none\" or \"required\"")),
    };

    let name = &f.sig.ident;
    let vis = &f.vis;
    let doc_attrs: Vec<_> = f
        .attrs
        .iter()
        .filter(|a| a.path().is_ident("doc"))
        .collect();
    let (description, idempotent, parallel_safe) =
        (&attrs.description, attrs.idempotent, attrs.parallel_safe);
    let states: Vec<&Type> = params
        .iter()
        .filter_map(|p| match p {
            Param::State(ty) => Some(ty),
            _ => None,
        })
        .collect();
    let fields: Vec<_> = (0..states.len()).map(|i| format_ident!("s{i}")).collect();
    let mut state_index = 0;
    let call_args: Vec<_> = params
        .iter()
        .map(|p| match p {
            Param::Context => quote!(_ctx),
            Param::Input(_) => quote!(_input),
            Param::State(_) => {
                let field = &fields[state_index];
                state_index += 1;
                quote!(::carmy::State(::core::clone::Clone::clone(&self.#field)))
            }
        })
        .collect();

    // The generated tool type: the marker itself when stateless, a hidden struct otherwise.
    let tool_ty = if states.is_empty() {
        quote!(#name)
    } else {
        let hidden = format_ident!("__CarmyTool_{}", name);
        quote!(#hidden)
    };
    let tool_impl = quote! {
        impl ::carmy::Tool for #tool_ty {
            type Input = #input;
            type Output = #output;
            fn metadata(&self) -> ::carmy::ToolMetadata {
                ::carmy::ToolMetadata {
                    name: stringify!(#name).into(),
                    description: #description.into(),
                    input_schema: ::carmy::schema::<#input>(),
                    output_schema: ::carmy::schema::<#output>(),
                    effect: ::carmy::Effect::#effect,
                    idempotent: #idempotent,
                    parallel_safe: #parallel_safe,
                    confirmation: ::carmy::Confirmation::#confirmation,
                }
            }
            #[allow(unused_variables)]
            async fn execute(
                &self,
                _ctx: ::carmy::AgentContext,
                _input: Self::Input,
            ) -> ::carmy::AgentResult<Self::Output> {
                #f
                #name(#(#call_args),*).await
            }
        }
    };
    let stateful = if states.is_empty() {
        quote!()
    } else {
        quote! {
            #[doc(hidden)]
            #[allow(non_camel_case_types)]
            #vis struct #tool_ty { #(#fields: #states),* }
            impl ::carmy::IntoTool for #name {
                type Tool = #tool_ty;
                fn into_tool(self, states: &::carmy::StateMap) -> ::carmy::AgentResult<#tool_ty> {
                    ::core::result::Result::Ok(#tool_ty {
                        #(#fields: states.get::<#states>(stringify!(#name))?),*
                    })
                }
            }
        }
    };
    let register = if attrs.register {
        let slot = format_ident!("__CARMY_REGISTER_{}", name.to_string().to_uppercase());
        quote! {
            #[::carmy::__private::linkme::distributed_slice(::carmy::__private::TOOLS)]
            #[linkme(crate = ::carmy::__private::linkme)]
            #[doc(hidden)]
            static #slot: fn(::carmy::Carmy) -> ::carmy::Carmy = |app| app.tool(#name);
        }
    } else {
        quote!()
    };
    Ok(quote! {
        #(#doc_attrs)*
        #[allow(non_camel_case_types)]
        #[derive(Clone, Copy, Debug, Default)]
        #vis struct #name;
        #stateful
        #tool_impl
        #register
    })
}
