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
instead of downloading it again.

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
- Responses are sent with `Cache-Control: no-store`.

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

Add `Accept: text/event-stream` to get [Server-Sent Events](/guides/streaming/). If the
client disconnects, the execution is cancelled.

## Embedding the router

```rust
let router: axum::Router = carmy::app().router()?;
// add your own routes and Tower middleware, then serve it with axum::serve
```
