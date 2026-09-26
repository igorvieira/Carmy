---
title: CLI
description: "The carmy command and the application commands."
sidebar:
  order: 2
---

## `carmy`

```console
cargo install carmy-cli
```

Carmy requires Rust **1.88** or newer (`rustup update stable`).

| command | effect |
|---------|--------|
| `carmy new <name>` | create an application with the [conventional layout](/getting-started/project-structure/) |
| `carmy new <name> --git` | depend on the `main` branch of the Git repository instead of crates.io |
| `carmy new <name> --path <dir>` | depend on a local Carmy checkout instead of crates.io |
| `carmy generate tool <name> --effect <effect>` | add a tool to the current application |
| `carmy g tool …` | shorthand for `carmy generate tool` |
| `carmy console` | explore and call the tools in a terminal UI ([Console](/guides/console/)) |
| `carmy console --jsonl` | speak `carmy-console/1` on stdio, for agents and scripts |
| `carmy server` | serve the application over HTTP |
| `carmy --version` | print the version |
| `carmy --help` | print usage |

`carmy new`:

- validates the name: lowercase ASCII letters, digits, `_` and `-`, starting with a
  letter; reserved names are refused
- never overwrites an existing directory
- runs `git init` when Git is available

## `carmy generate tool`

```console
carmy g tool create_order --effect write --description "Place an order"
```

It creates `src/tools/create_order.rs`, with typed input and output structs, the
`#[carmy::tool]` function and a test, and declares `mod create_order;` in
`src/tools/mod.rs`, which registers the tool. It works from any directory inside the
application.

| option | meaning |
|--------|---------|
| `--effect` | **required**: `none`, `read`, `write`, `external_write` or `destructive` |
| `--description` | what the tool does; agents read it |

The attributes follow the effect:

| effect | attributes |
|--------|------------|
| `none`, `read` | `idempotent` and `parallel_safe` |
| `destructive` | `confirmation = "required"`, and the generated test checks that confirmation is enforced |

`carmy g tool` never overwrites a file, rejects names that are not snake_case (and Rust
keywords), and formats the code with `rustfmt` when it is installed.

## Application commands

A Carmy application built with `carmy::run()` accepts:

| command | effect |
|---------|--------|
| `cargo run` or `cargo run -- server` | serve HTTP |
| `cargo run -- mcp` | serve MCP over stdio |
| `cargo run -- tools` | print the tool catalog as JSON |
| `cargo run -- console` | serve `carmy-console/1` on stdio |

The same commands work on the compiled binary: `./target/release/shop mcp`.
