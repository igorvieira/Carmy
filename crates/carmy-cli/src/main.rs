use carmy_cli::{Dependency, generate};
use std::{path::PathBuf, process::ExitCode};

const USAGE: &str = "\
Carmy: agent-native execution infrastructure for Rust

Usage:
  carmy new <name> [--path <carmy-checkout>]   Create a new application
  carmy --version

Options:
  --path <dir>   Depend on a local Carmy checkout instead of the Git repository";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["new", name] => new(name, Dependency::Git),
        ["new", name, "--path", path] | ["new", "--path", path, name] => {
            new(name, Dependency::Path(PathBuf::from(path)))
        }
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

Add tools in src/tools/ (one file per tool, declared in src/tools/mod.rs)."
    );
    ExitCode::SUCCESS
}
