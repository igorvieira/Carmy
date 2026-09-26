---
title: MCP
description: "Serve the same tools to MCP clients such as Claude Desktop and Cursor."
sidebar:
  order: 2
---

`carmy-mcp` adapts the runtime to the Model Context Protocol using the official Rust SDK,
[`rmcp`](https://github.com/modelcontextprotocol/rust-sdk). Your tools don't change.

```console
cargo run -- mcp
```

## Connecting a client

Build a release binary, then point your MCP client at it. For example, in Claude Desktop's
configuration:

```json
{
  "mcpServers": {
    "shop": { "command": "/path/to/shop/target/release/shop", "args": ["mcp"] }
  }
}
```

Logs go to stderr, so stdout stays reserved for the protocol.

## Supported in v0.1

| MCP feature | support |
|-------------|---------|
| `initialize`, `ping` | yes |
| `tools/list` | yes, with the full catalog in one page and `ttlMs` of 60000 |
| `tools/call` | yes |
| `notifications/cancelled` | yes; cancels the execution |
| stdio transport | yes |
| other `rmcp` transports | yes, through `rmcp::ServiceExt::serve` |
| resources, prompts, completion | no |
| sampling, elicitation, roots | no |
| progress notifications, tasks | no |

## Mapping

| Carmy | MCP |
|-------|-----|
| `effect` `none` or `read` | `readOnlyHint: true` |
| `effect` `destructive` | `destructiveHint: true` |
| `effect` `external_write` | `openWorldHint: true` |
| `idempotent` | `idempotentHint` |
| effect, confirmation, parallel safety | `_meta["carmy/effect"]`, `_meta["carmy/confirmation"]`, `_meta["carmy/parallel_safe"]` |
| successful object result | `structuredContent`, plus JSON text content |
| `AgentError` | `isError: true`, with `{"error": …}` as JSON text content and in `_meta["carmy/error"]`; never in `structuredContent`, which clients validate against the output schema |
| unknown tool | JSON-RPC `-32602`, with `data.error` |
| `_meta["carmy/request_id"]` on `tools/call` | the idempotency `request_id` |
| execution ID and status | result `_meta["carmy/execution_id"]` and `_meta["carmy/status"]`: always on errors, on successes with `McpServer::execution_meta(true)` |

## Explicit setup

```rust
let runtime = Carmy::new().tool(search).build()?;
carmy::mcp::McpServer::new(runtime)
    .context(trusted_context) // e.g. confirmation grants decided by the host
    .serve_stdio()
    .await?;
```

## Limitations

- MCP expects object input schemas, so tools whose input is not an object are listed
  as-is.
- Confirmation-required tools need a host-granted `confirm:<tool>` in the connection's
  context. There is no per-call approval flow through elicitation yet.
