---
title: Streaming
description: "Follow executions as a stream of lifecycle events, in-process or as SSE."
sidebar:
  order: 6
---

The runtime exposes every execution as a stream of events, independent of any wire
format:

| event | when |
|-------|------|
| `execution.started` | always first |
| `tool.started` | the tool is about to run (after policies, validation and idempotency) |
| `tool.completed` | the tool finished, with `duration_ms` and `ok` |
| `execution.completed` | always last, with the full result and `replayed` |

Replays and early rejections skip the `tool.*` events.

## Over HTTP (SSE)

Send the normal execute request with `Accept: text/event-stream`:

```console
$ curl -N localhost:3000/agent/execute -H 'content-type: application/json' \
    -H 'accept: text/event-stream' -d '{"tool":"search","arguments":{"query":"mouse"}}'
event: execution.started
data: {"execution_id":"exec_…","tool":"search"}

event: tool.started
data: {"execution_id":"exec_…","tool":"search"}

event: tool.completed
data: {"duration_ms":0,"execution_id":"exec_…","ok":true,"tool":"search"}

event: execution.completed
data: {"_agent":{"cacheable":true,"next_actions":[]},"data":{…},"execution_id":"exec_…","replayed":false,"status":"completed"}
```

The `execution.completed` payload is the same body a JSON response would have, plus
`replayed`.

## In-process

```rust
use futures_util::StreamExt;

let runtime = carmy::app().build()?;
let request = carmy::runtime::execution_request("search", serde_json::json!({ "query": "x" }));
let mut events = runtime.execute_stream(request);
while let Some(event) = events.next().await {
    println!("{}", event.name());
}
```

The stream drives the execution itself; nothing is spawned in the background. Dropping
the stream (for example, when an SSE client disconnects) cancels the execution, exactly
like dropping a normal request.
