use carmy_cli::{Dependency, find_project, generate, generate_tool};
use std::{path::PathBuf, process::ExitCode};

const USAGE: &str = "\
Carmy: agent-native execution infrastructure for Rust

Usage:
  carmy new <name> [--git | --path <carmy-checkout>]   Create a new application
  carmy generate tool <name> --effect <effect> [--description <text>]
                                                  Add a tool to the current application
  carmy g tool ...                                Shorthand for `generate tool`
  carmy --version

Options for `new`:
  --git          Depend on the main branch of the Git repository instead of crates.io
  --path <dir>   Depend on a local Carmy checkout instead of crates.io

Options for `generate tool`:
  --effect <effect>       none, read, write, external_write or destructive (required)
  --description <text>    What the tool does, for agents";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["new", name] => new(name, Dependency::CratesIo),
        ["new", name, "--git"] | ["new", "--git", name] => new(name, Dependency::Git),
        ["new", name, "--path", path] | ["new", "--path", path, name] => {
            new(name, Dependency::Path(PathBuf::from(path)))
        }
        ["generate" | "g", "tool", name, rest @ ..] => tool(name, rest),
        ["--version" | "-V"] => {
            println!("carmy {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        [] | ["help" | "--help" | "-h"] => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn tool(name: &str, flags: &[&str]) -> ExitCode {
    let (mut effect, mut description) = (None, None);
    let mut flags = flags.iter();
    while let Some(flag) = flags.next() {
        match (*flag, flags.next()) {
            ("--effect", Some(value)) => effect = Some(*value),
            ("--description", Some(value)) => description = Some(*value),
            _ => {
                eprintln!("error: unexpected argument `{flag}`\n\n{USAGE}");
                return ExitCode::FAILURE;
            }
        }
    }
    let Some(effect) = effect else {
        eprintln!(
            "error: declare the tool's side effect with --effect: none, read, write, external_write or destructive"
        );
        return ExitCode::FAILURE;
    };
    let description = description.unwrap_or("Describe what this tool does, for agents");
    let result = std::env::current_dir()
        .map_err(carmy_cli::Error::from)
        .and_then(|dir| find_project(&dir))
        .and_then(|project| generate_tool(&project, name, effect, description));
    match result {
        Ok(file) => {
            println!(
                "Created {}\nDeclared `mod {name};` in src/tools/mod.rs\n\nThe tool registers itself; `cargo test` runs its test.",
                file.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn new(name: &str, carmy: Dependency) -> ExitCode {
    let root = match generate(&PathBuf::from("."), name, &carmy) {
        Ok(root) => root,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Like `cargo new`: start a repository when git is available; skip silently otherwise.
    let _ = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&root)
        .status();
    println!(
        "Created {name}

  cd {name}
  cargo run              # HTTP on http://127.0.0.1:3000/.well-known/agent
  cargo run -- mcp       # MCP over stdio
  cargo test

Add a tool: carmy g tool search --effect read --description \"Search the catalog\""
    );
    ExitCode::SUCCESS
}
