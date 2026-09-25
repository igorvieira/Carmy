# {{name}}

A [Carmy](https://github.com/igorvieira/carmy) application.

```console
cargo run              # HTTP on 127.0.0.1:3000 (see carmy.toml)
cargo run -- mcp       # MCP over stdio (Claude Desktop, Cursor, ...)
cargo run -- tools     # print the tool catalog
cargo test
```

## Layout

| path              | purpose                                                    |
|-------------------|------------------------------------------------------------|
| `src/main.rs`     | entry point; register shared state with `.state(..)`       |
| `src/tools/`      | one file per tool, declared in `src/tools/mod.rs`          |
| `carmy.toml`      | name, address and timeout (overridden by `CARMY_*` env)    |

## Adding a tool

1. Create `src/tools/search.rs` with an `#[carmy::tool]` function.
2. Add `mod search;` to `src/tools/mod.rs`.

That's it: the tool is discovered over HTTP and MCP.

```sh
curl localhost:3000/agent/tools
curl localhost:3000/agent/execute -H 'content-type: application/json' \
  -d '{"tool":"hello","arguments":{"name":"Ada"}}'
```
