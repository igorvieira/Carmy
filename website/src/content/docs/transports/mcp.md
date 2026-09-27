---
title: MCP
description: "Serve the same tools to MCP clients such as Claude Desktop and Cursor, over stdio or HTTP."
sidebar:
  order: 2
---

`carmy-mcp` adapts the runtime to the Model Context Protocol using the official Rust SDK,
[`rmcp`](https://github.com/modelcontextprotocol/rust-sdk). Your tools don't change.

## Over stdio

```console
cargo run -- mcp
```

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

## Over HTTP

With the `http` and `mcp` features (both on by default), the app serves MCP over
Streamable HTTP at `/mcp`, next to the agent routes and behind the same connection
limits and timeouts. Discovery advertises it as `mcp_url`.

```toml
[mcp]
http = true                          # false: stdio only
path = "/mcp"
allowed_hosts = ["api.example.com"]  # CARMY_MCP_ALLOWED_HOSTS=a,b
```

`allowed_hosts` guards against DNS rebinding by checking the `Host` header. Without
it, only localhost is accepted, so **list your public hosts in production**.

Calls take the `AgentContext` your authentication middleware put on the request, the
same one [HTTP](/transports/http/#embedding-the-router) calls use:

```rust
let router = carmy::app().router()?.layer(from_fn(authenticate)); // inserts Extension<AgentContext>
```

## Progress

When a `tools/call` carries a `progressToken`, what the tool reports through
[`ctx.progress`](/guides/streaming/#progress) arrives as `notifications/progress`, with
the progress, the total and the message. Partial results are not part of MCP progress;
they stay on Carmy's own streams (SSE, the console).

## Tasks

With clients that support [tasks](https://modelcontextprotocol.io) (the
`io.modelcontextprotocol/tasks` extension), a call still running after
`promote_after` becomes a task: the client gets a task id at once, polls it with
`tasks/get` and may cancel it with `tasks/cancel`. The tool's progress messages become
the task's status message. Fast calls, and clients without tasks, answer inline as
always.

```toml
[mcp]
promote_after_ms = 2000   # default; 0 never promotes
```

These tasks live in the process for five minutes. For work that must survive a
restart, enqueue a [job](/guides/jobs/) and follow it with `carmy_job`.

## Confirmation

A tool that needs confirmation (destructive, or `confirmation = "required"`) asks the
person behind the client, through an elicitation form: "`delete_customer` needs your
confirmation before it runs with {…}. Allow it?". On a yes, the call runs with
`confirm:<tool>` for that call only. Clients that cannot elicit get the usual
`CONFIRMATION_REQUIRED` error. Turn the question off when the client's answer must not
count:

```toml
[mcp]
confirm_by_elicitation = false
```

## Supported

| MCP feature | support |
|-------------|---------|
| `initialize`, `ping` | yes |
| `tools/list` | yes, the full catalog in one page, `ttlMs` of 60000 |
| `tools/call` | yes |
| `notifications/cancelled` | yes; cancels the execution |
| `notifications/progress` | yes, for calls with a `progressToken` |
| tasks (`tasks/get`, `tasks/cancel`) | yes, for calls outliving `promote_after` |
| elicitation | yes, to confirm tools that need it |
| stdio, Streamable HTTP | yes |
| resources, prompts, completion, sampling | no |

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
| execution ID and status | `_meta["carmy/execution_id"]` and `_meta["carmy/status"]`: always on errors, on successes with `McpServer::execution_meta(true)` |
| [next actions](/transports/http/#next-actions) | `_meta["carmy/next_actions"]`, whenever there are some |

## Explicit setup

```rust
let runtime = Carmy::new().tool(search).build()?;
let server = carmy::mcp::McpServer::new(runtime)
    .context(trusted_context)          // e.g. grants decided by the host
    .promote_after(Some(Duration::from_secs(5)))
    .confirm_by_elicitation(true);
server.clone().serve_stdio().await?;                    // stdio
let service = server.http_service(Some(vec!["api.example.com".into()]));
let router = axum::Router::new().nest_service("/mcp", service);  // HTTP, feature `http`
```

## Limitations

- MCP expects object input schemas, so tools whose input is not an object are listed
  as-is.
- Tasks are in-process; a restart loses them.
- MCP sessions over HTTP are in memory: behind a load balancer, keep a client on one
  instance (sticky sessions), or run MCP over stdio.
