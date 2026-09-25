---
title: CLI
description: "The carmy command and the application commands."
sidebar:
  order: 2
---

## `carmy`

```console
cargo install --git https://github.com/igorvieira/Carmy carmy-cli
```

| command | effect |
|---------|--------|
| `carmy new <name>` | create an application with the [conventional layout](/getting-started/project-structure/) |
| `carmy new <name> --path <dir>` | depend on a local Carmy checkout instead of the Git repository |
| `carmy --version` | print the version |
| `carmy --help` | print usage |

`carmy new`:

- validates the name: lowercase ASCII letters, digits, `_` and `-`, starting with a
  letter; reserved names are refused
- never overwrites an existing directory
- runs `git init` when Git is available

## Application commands

A Carmy application built with `carmy::run()` accepts:

| command | effect |
|---------|--------|
| `cargo run` or `cargo run -- server` | serve HTTP |
| `cargo run -- mcp` | serve MCP over stdio |
| `cargo run -- tools` | print the tool catalog as JSON |

The same commands work on the compiled binary: `./target/release/shop mcp`.
