---
title: HTTP
description: "The carmy/1 HTTP protocol: discovery, catalog, execution and SSE."
sidebar:
  order: 1
---

The HTTP adapter serves three endpoints over Axum. All bodies are JSON, and request bodies
are limited to 1 MiB.

## `GET /.well-known/agent`

```json
{
  "protocol": "carmy/1",
  "server": "shop",
  "capabilities": ["tools", "streaming", "idempotency"],
  "tools_url": "/agent/tools",
  "tools_version": "3762507c…",
  "execute_url": "/agent/execute"
}
```

`tools_version` changes only when the catalog changes, so agents can cache the catalog
instead of downloading it again. An app with [webhooks](/guides/webhooks/) adds
`"webhooks"` to `capabilities` and lists each one (path, tool, mode, identity pointer),
never with its secret. With [MCP over HTTP](/transports/mcp/#over-http), `mcp_url` points
at it and `capabilities` includes `"mcp"`.

## `GET /agent/tools`

```json
{
  "tools": [
    {
      "name": "create_order",
      "description": "Place an order",
      "input_schema": { "type": "object", "properties": { "sku": { "type": "string" } }, "required": ["sku"] },
      "output_schema": { "type": "object", "properties": { "order_id": { "type": "integer" } } },
      "effect": "write",
      "idempotent": false,
      "parallel_safe": true,
      "confirmation": "none"
    }
  ]
}
```

Both discovery documents are serialized once at startup and served with an `ETag` and
`Cache-Control: private, max-age=60`. A request with `If-None-Match` gets `304 Not
Modified` when the document hasn't changed.

## `POST /agent/execute`

```json
{ "tool": "create_order", "arguments": { "sku": "KB-01" }, "request_id": "order-7f3" }
```

- `arguments` defaults to `{}`.
- `request_id` is optional. Send it for anything you might [retry](/guides/idempotency/).
- Unknown fields are rejected. A request can never carry context or permissions.

```json
{
  "execution_id": "exec_…",
  "status": "completed",
  "data": { "order_id": 1 },
  "_agent": { "cacheable": false, "next_actions": [] }
}
```

- `status` is one of `completed`, `failed`, `cancelled` or `timed_out`.
- Failures carry [`error`](/guides/errors/) instead of `data`.
- `_agent.cacheable` is `true` when a result can be reused: a successful call to an
  idempotent tool with a `read` or `none` effect.
- `_agent.next_actions` lists what to do next, when Carmy knows it; see below.
- Responses are sent with `Cache-Control: no-store`.

### Next actions

```json
"_agent": { "cacheable": false, "next_actions": [
  { "tool": "publish", "reason": "Retry the same call with the same request_id", "after_ms": 5000 }
] }
```

| after | Carmy suggests |
|-------|----------------|
| a retryable error | the same call, same `request_id`, after `retry_after` |
| `CONFIRMATION_REQUIRED` | the same call, once a person confirmed |
| `carmy_job` for a job still queued or running | `carmy_job` again, in a second |
| a webhook delivery that was queued (`202`) | `carmy_job` with the new `job_id` |

`arguments` absent means the same arguments. An empty list means there is nothing to
suggest. The same list travels in the console's answers and in MCP results'
`_meta["carmy/next_actions"]`.

### Status codes

| category | status | | category | status |
|----------|--------|-|----------|--------|
| `validation` | 400 | | `conflict` | 409 |
| `permission` | 403 | | `internal` | 500 |
| `not_found` | 404 | | `capacity` | 503 |
| `cancelled` | 408 | | `timeout` | 504 |

The body is authoritative; the status code is a courtesy for generic HTTP tooling.
Malformed bodies return `400 INVALID_REQUEST` with `details.reason`, and oversized bodies
return `413 PAYLOAD_TOO_LARGE`.

## Streaming

Add `Accept: text/event-stream` to get [Server-Sent Events](/guides/streaming/), including
the tool's `tool.progress`. If the client disconnects, the execution is cancelled.

## MCP

With the `mcp` feature, the same router serves [MCP over HTTP](/transports/mcp/#over-http)
at `/mcp`; discovery lists it as `mcp_url`.

## Embedding the router

```rust
let router: axum::Router = carmy::app().router()?;
// add your own routes and Tower middleware, then serve it with axum::serve
```
