//! `carmy generate tool`: one file per tool in `src/tools/`, declared in `src/tools/mod.rs`.
use crate::Error;
use std::{
    fs,
    path::{Path, PathBuf},
};

const TEMPLATE: &str = include_str!("../templates/tool.rs.tpl");
const EFFECTS: [&str; 5] = ["none", "read", "write", "external_write", "destructive"];
const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use",
    "where", "while", "gen", "abstract", "become", "box", "do", "final", "macro", "override",
    "priv", "try", "typeof", "unsized", "virtual", "yield", "tests",
];

/// A tool name is its Rust function name and its wire name: snake_case.
pub fn validate_tool_name(name: &str) -> Result<(), Error> {
    let invalid = |reason: &str| Err(Error::InvalidName(format!("`{name}` {reason}")));
    if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
        return invalid("must start with a lowercase ASCII letter");
    }
    if name.len() > 64 {
        return invalid("must be at most 64 characters");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return invalid("must be snake_case: a-z, 0-9 and `_`");
    }
    if KEYWORDS.contains(&name) {
        return invalid("is a Rust keyword");
    }
    Ok(())
}

/// The nearest ancestor of `start` that is a Carmy application (has `src/tools/mod.rs`).
pub fn find_project(start: &Path) -> Result<PathBuf, Error> {
    start
        .ancestors()
        .find(|dir| dir.join("Cargo.toml").is_file() && dir.join("src/tools/mod.rs").is_file())
        .map(Path::to_path_buf)
        .ok_or_else(|| Error::NotAProject(start.to_path_buf()))
}

fn pascal_case(name: &str) -> String {
    name.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect()
}

fn render(name: &str, effect: &str, description: &str) -> String {
    let description = description.replace('\\', "\\\\").replace('"', "\\\"");
    let mut attributes = vec![
        format!("description = \"{description}\""),
        format!("effect = \"{effect}\""),
    ];
    // Sensible defaults per effect; every attribute stays visible in the generated code.
    let (test, assertion) = match effect {
        "none" | "read" => {
            attributes.push("idempotent = true".into());
            attributes.push("parallel_safe = true".into());
            (
                "runs",
                "        assert_eq!(result.outcome.unwrap()[\"value\"], \"hello\");",
            )
        }
        "destructive" => {
            attributes.push("confirmation = \"required\"".into());
            (
                "requires_confirmation",
                "        // Destructive tools only run with a trusted `confirm:<tool>` grant.\n        assert_eq!(result.outcome.unwrap_err().code, \"CONFIRMATION_REQUIRED\");",
            )
        }
        _ => (
            "runs",
            "        assert_eq!(result.outcome.unwrap()[\"value\"], \"hello\");",
        ),
    };
    TEMPLATE
        .replace("{{Pascal}}", &pascal_case(name))
        .replace("{{attribute}}", &tool_attribute(&attributes))
        .replace("{{assertion}}", assertion)
        .replace("{{test}}", &format!("{name}_{test}"))
        .replace("{{name}}", name)
}

/// `#[carmy::tool(...)]` laid out like rustfmt lays out call arguments: one line when the
/// arguments fit in 60 columns, one argument per line otherwise.
fn tool_attribute(arguments: &[String]) -> String {
    let inline = arguments.join(", ");
    if inline.len() <= 60 {
        format!("#[carmy::tool({inline})]")
    } else {
        format!("#[carmy::tool(\n    {}\n)]", arguments.join(",\n    "))
    }
}

/// Adds `mod name;` to the declarations, keeping consecutive `mod` lines sorted the way
/// `rustfmt` orders them. Other lines stay where they are.
fn declare(declarations: &str, name: &str) -> String {
    let is_mod = |line: &str| {
        line.strip_prefix("mod ")
            .and_then(|rest| rest.strip_suffix(';'))
            .is_some_and(|ident| ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
    };
    let lines: Vec<&str> = declarations.lines().collect();
    let declaration = format!("mod {name};");
    let Some(first) = lines.iter().position(|l| is_mod(l)) else {
        let mut out = declarations.to_owned();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        return out + &declaration + "\n";
    };
    let end = lines[first..]
        .iter()
        .position(|l| !is_mod(l))
        .map_or(lines.len(), |offset| first + offset);
    let mut block: Vec<&str> = lines[first..end].to_vec();
    block.push(&declaration);
    block.sort_unstable();
    let mut out: Vec<&str> = lines[..first].to_vec();
    out.extend(block);
    out.extend(&lines[end..]);
    out.join("\n") + "\n"
}

/// Create `src/tools/<name>.rs` and declare it in `src/tools/mod.rs`. Returns the new file.
pub fn generate_tool(
    project: &Path,
    name: &str,
    effect: &str,
    description: &str,
) -> Result<PathBuf, Error> {
    validate_tool_name(name)?;
    if !EFFECTS.contains(&effect) {
        return Err(Error::InvalidEffect(effect.to_owned()));
    }
    let tools = project.join("src/tools");
    let file = tools.join(format!("{name}.rs"));
    let module = tools.join("mod.rs");
    let declarations = fs::read_to_string(&module)?;
    let declaration = format!("mod {name};");
    if file.exists() || declarations.lines().any(|l| l.trim() == declaration) {
        return Err(Error::Exists(file));
    }
    fs::write(&file, render(name, effect, description))?;
    fs::write(&module, declare(&declarations, name))?;
    // Match the project's own formatter when rustfmt is installed (it ships with rustup).
    let _ = std::process::Command::new("rustfmt")
        .args(["--edition", "2024"])
        .arg(&file)
        .arg(&module)
        .stderr(std::process::Stdio::null())
        .status();
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pascal_case_names_the_types() {
        assert_eq!(pascal_case("create_order"), "CreateOrder");
        assert_eq!(pascal_case("ping"), "Ping");
        assert_eq!(pascal_case("get_user_v2"), "GetUserV2");
    }
    #[test]
    fn declarations_stay_sorted() {
        let doc = "//! Tools.\nmod hello;\n";
        assert_eq!(
            declare(doc, "create"),
            "//! Tools.\nmod create;\nmod hello;\n"
        );
        assert_eq!(declare(doc, "zeta"), "//! Tools.\nmod hello;\nmod zeta;\n");
        assert_eq!(declare("//! Tools.\n", "a"), "//! Tools.\nmod a;\n");
    }
    #[test]
    fn descriptions_are_escaped() {
        let code = render("x", "read", r#"Say "hi" \ bye"#);
        assert!(
            code.contains(r#"description = "Say \"hi\" \\ bye""#),
            "{code}"
        );
    }
}
