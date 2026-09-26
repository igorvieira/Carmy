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

```console
carmy g tool search --effect read --description "Search the catalog"
```

This creates `src/tools/search.rs`, with a test, and declares it in `src/tools/mod.rs`.
The tool registers itself and is discovered over HTTP and MCP.

```sh
curl localhost:3000/agent/tools
curl localhost:3000/agent/execute -H 'content-type: application/json' \
  -d '{"tool":"hello","arguments":{"name":"Ada"}}'
```
