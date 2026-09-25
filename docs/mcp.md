# MCP adapter

`carmy-mcp` exposes a Carmy runtime as an MCP server using the official Rust SDK,
[`rmcp`](https://github.com/modelcontextprotocol/rust-sdk). Protocol version negotiation
is handled by `rmcp`.

```rust
let runtime = Carmy::new().tool(search).build()?;
carmy::mcp::McpServer::new(runtime).serve_stdio().await?;
// or: Carmy::new().tool(search).serve_mcp_stdio().await?
```

`McpServer` implements `rmcp::ServerHandler`, so it can be served over any `rmcp`
transport.

## Supported in v0.1

| MCP feature                       | support |
|-----------------------------------|---------|
| `initialize`, `ping`              | yes |
| `tools/list`                      | yes, with the full catalog in one page and `ttlMs` of 60000 |
| `tools/call`                      | yes |
| `notifications/cancelled`         | yes; cancels the execution's token |
| stdio transport                   | yes (`serve_stdio`) |
| other `rmcp` transports           | yes, through `rmcp::ServiceExt::serve` |
| resources, prompts, completion    | no |
| sampling, elicitation, roots      | no |
| progress notifications, tasks     | no |
| `tools/list_changed`              | no; the catalog is fixed at startup |

## Mapping

| Carmy                                    | MCP |
|------------------------------------------|-----|
| `effect` `none` or `read`                | `readOnlyHint: true` |
| `effect` `destructive`                   | `destructiveHint: true` |
| `effect` `external_write`                | `openWorldHint: true` |
| `idempotent`                             | `idempotentHint` |
| `effect`, `confirmation`, `parallel_safe` | `_meta["carmy/effect"]`, `_meta["carmy/confirmation"]`, `_meta["carmy/parallel_safe"]` |
| input schema                             | `inputSchema` |
| object output schema                     | `outputSchema` |
| successful object result                 | `structuredContent`, plus JSON text content |
| successful non-object result             | JSON text content only |
| `AgentError`                             | `isError: true`, with `structuredContent: {"error": AgentError}` |
| unknown tool                             | JSON-RPC `-32602`, with `data.error` holding the `AgentError` |
| `_meta["carmy/request_id"]` on `tools/call` | idempotency `request_id` |
| execution ID and status                  | result `_meta["carmy/execution_id"]` and `_meta["carmy/status"]` |

## Limitations

- MCP expects object input schemas, so tools whose input is not an object are listed as-is.
- Confirmation-required tools need a host-granted `confirm:<tool>` permission in `McpServer::context`. There is no per-call approval flow through elicitation yet.
