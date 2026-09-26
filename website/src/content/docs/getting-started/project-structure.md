---
title: Project structure
description: "The conventional layout of a Carmy application."
sidebar:
  order: 3
---

`carmy new` creates this layout:

```text
shop/
├── Cargo.toml
├── carmy.toml          # name, address, timeout; CARMY_* env vars override
├── README.md
├── .gitignore
└── src/
    ├── main.rs         # the entry point
    └── tools/
        ├── mod.rs      # one `mod` line per tool
        └── hello.rs    # a tool and its tests
```

## `src/main.rs`

```rust
mod tools;

#[tokio::main]
async fn main() -> carmy::Result {
    carmy::run().await
}
```

`carmy::run()` is shorthand for `carmy::app().run().await`. `carmy::app()` does three
things:

- reads [configuration](/guides/configuration/) from `carmy.toml` and the environment
- registers every tool declared with `#[carmy::tool]`
- chooses the transport from the first command-line argument

To register shared dependencies, use the builder:

```rust
#[tokio::main]
async fn main() -> carmy::Result {
    let db = Db::connect("postgres://localhost/shop").await.expect("database");
    carmy::app().state(db).run().await
}
```

## `src/tools/`

The convention is one file per tool, declared in `src/tools/mod.rs`. The generator does
both for you:

```console
carmy g tool search --effect read --description "Search the catalog"
```

The result looks like this:

```rust
//! One file per tool. Declare each file here; `#[carmy::tool]` registers it.
mod hello;
mod search;
```

Keep a tool's tests in the same file (see [Testing](/guides/testing/)).

## `carmy.toml`

```toml
name = "shop"                # CARMY_NAME
address = "127.0.0.1:3000"   # CARMY_ADDR
timeout_secs = 30            # CARMY_TIMEOUT_SECS
```

## Commands

| command | effect |
|---------|--------|
| `cargo run` or `cargo run -- server` | serve HTTP on the configured address |
| `cargo run -- mcp` | serve MCP over stdin/stdout |
| `cargo run -- tools` | print the tool catalog as JSON |
| `cargo run -- console` or `carmy console` | explore and call the tools (see [Console](/guides/console/)) |
| `cargo test` | run the tests |
