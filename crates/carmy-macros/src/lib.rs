//! Attribute macro implementing Carmy's typed Tool trait.
use proc_macro::TokenStream;
use quote::quote;
use syn::{FnArg, GenericArgument, ItemFn, PathArguments, ReturnType, Type, parse_macro_input};

#[proc_macro_attribute]
pub fn tool(args: TokenStream, input: TokenStream) -> TokenStream {
    let function = parse_macro_input!(input as ItemFn);
    match expand(args.into(), function) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}
fn expand(args: proc_macro2::TokenStream, f: ItemFn) -> syn::Result<proc_macro2::TokenStream> {
    let mut description = String::new();
    let mut effect = None;
    let mut idempotent = false;
    let mut parallel_safe = false;
    let mut confirmation = "none".to_string();
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("description") {
            description = meta.value()?.parse::<syn::LitStr>()?.value();
        } else if meta.path.is_ident("effect") {
            effect = Some(meta.value()?.parse::<syn::LitStr>()?.value());
        } else if meta.path.is_ident("idempotent") {
            idempotent = meta.value()?.parse::<syn::LitBool>()?.value;
        } else if meta.path.is_ident("parallel_safe") {
            parallel_safe = meta.value()?.parse::<syn::LitBool>()?.value;
        } else if meta.path.is_ident("confirmation") {
            confirmation = meta.value()?.parse::<syn::LitStr>()?.value();
        } else {
            return Err(meta.error("unknown Carmy tool attribute"));
        }
        Ok(())
    });
    syn::parse::Parser::parse2(parser, args)?;
    let err = |message| syn::Error::new_spanned(&f.sig, message);
    if f.sig.asyncness.is_none()
        || f.sig.inputs.len() != 2
        || !f.sig.generics.params.is_empty()
        || f.sig.unsafety.is_some()
        || f.sig.abi.is_some()
    {
        return Err(err(
            "tool must be a safe async fn(context: AgentContext, input: Input) -> AgentResult<Output> without generics",
        ));
    }
    let input = match &f.sig.inputs[1] {
        FnArg::Typed(arg) => &arg.ty,
        _ => return Err(err("tool cannot take self")),
    };
    let output = match &f.sig.output {
        ReturnType::Type(_, ty) => match ty.as_ref() {
            Type::Path(path) => {
                let segment = path.path.segments.last().unwrap();
                match (&segment.ident.to_string()[..], &segment.arguments) {
                    ("AgentResult", PathArguments::AngleBracketed(args))
                        if args.args.len() == 1 =>
                    {
                        match &args.args[0] {
                            GenericArgument::Type(ty) => ty,
                            _ => return Err(err("expected AgentResult<Output>")),
                        }
                    }
                    _ => return Err(err("expected AgentResult<Output>")),
                }
            }
            _ => return Err(err("expected AgentResult<Output>")),
        },
        _ => return Err(err("expected AgentResult<Output>")),
    };
    let effect = match effect.as_deref() {
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
    let confirmation = match confirmation.as_str() {
        "none" => quote!(None),
        "required" => quote!(Required),
        _ => return Err(err("confirmation must be \"none\" or \"required\"")),
    };
    let name = &f.sig.ident;
    let vis = &f.vis;
    let attrs = &f.attrs;
    Ok(quote! {
        #(#attrs)*
        #[allow(non_camel_case_types)]
        #vis struct #name;
        impl ::carmy::Tool for #name {
            type Input = #input;
            type Output = #output;
            fn metadata(&self) -> ::carmy::ToolMetadata {
                ::carmy::ToolMetadata {
                    name: stringify!(#name).into(), description: #description.into(),
                    input_schema: ::carmy::schema::<#input>(), output_schema: ::carmy::schema::<#output>(),
                    effect: ::carmy::Effect::#effect, idempotent: #idempotent, parallel_safe: #parallel_safe,
                    confirmation: ::carmy::Confirmation::#confirmation,
                }
            }
            async fn execute(&self, ctx: ::carmy::AgentContext, input: Self::Input) -> ::carmy::AgentResult<Self::Output> {
                #f
                #name(ctx, input).await
            }
        }
    })
}
