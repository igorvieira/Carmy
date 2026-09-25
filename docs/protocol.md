# HTTP protocol (`carmy/1`)

All bodies are JSON. The request body limit is 1 MiB.

## `GET /.well-known/agent`

```json
{
  "protocol": "carmy/1",
  "server": "tool-server",
  "capabilities": ["tools", "streaming", "idempotency"],
  "tools_url": "/agent/tools",
  "tools_version": "3762507c…",
  "execute_url": "/agent/execute"
}
```

`tools_version` changes whenever the catalog changes. Agents can cache the catalog and
skip refetching it while the version stays the same.

## `GET /agent/tools`

```json
{
  "tools": [
    {
      "name": "create_order",
      "description": "Place an order for one product.",
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

Both discovery documents are sent with `ETag` and `Cache-Control: private, max-age=60`.
A request with `If-None-Match` gets `304 Not Modified` when the document is unchanged.

## `POST /agent/execute`

The request body:

```json
{ "tool": "create_order", "arguments": { "sku": "KB-01" }, "request_id": "order-7f3" }
```

- `arguments` defaults to `{}`.
- `request_id` is optional. Send it for any call you might retry.
- Unknown fields are rejected. In particular, a request cannot carry context or permissions. Those come from host middleware, through an `Extension<AgentContext>`.

The response:

```json
{
  "execution_id": "exec_…",
  "status": "completed",
  "data": { "order_id": 1 },
  "_agent": { "cacheable": false, "next_actions": [] }
}
```

`status` is one of `completed`, `failed`, `cancelled` or `timed_out`. Failures carry
`error` (see the README) instead of `data`. A replayed response is byte-identical to the
original, including `execution_id`. The response has `Cache-Control: no-store`.

HTTP status by error category:

| category     | status |
|--------------|--------|
| `validation` | 400    |
| `permission` | 403    |
| `not_found`  | 404    |
| `cancelled`  | 408    |
| `conflict`   | 409    |
| `internal`   | 500    |
| `capacity`   | 503    |
| `timeout`    | 504    |

The body is authoritative; the status code is a courtesy for generic HTTP tooling.
Malformed bodies return `400 INVALID_REQUEST` with `details.reason`, and oversized bodies
return `413 PAYLOAD_TOO_LARGE`.

## Streaming

To stream, send the same request with `Accept: text/event-stream`:

```text
event: execution.started
data: {"execution_id":"exec_…","tool":"search_products"}

event: tool.started
data: {"execution_id":"exec_…","tool":"search_products"}

event: tool.completed
data: {"execution_id":"exec_…","tool":"search_products","duration_ms":0,"ok":true}

event: execution.completed
data: {"execution_id":"exec_…","status":"completed","data":{…},"_agent":{…},"replayed":false}
```

`execution.completed` is always the final event. Its payload is the JSON response body
plus `replayed`. If the client disconnects before it arrives, the execution is cancelled.
