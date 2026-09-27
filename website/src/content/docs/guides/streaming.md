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
| `tool.progress` | the tool reported progress or a partial result; as often as it does |
| `tool.completed` | the tool finished, with `duration_ms` and `ok` |
| `execution.completed` | always last, with the full result and `replayed` |

Replays and early rejections skip the `tool.*` events.

## Progress

Tools report through `ctx.progress`. Nobody listening makes it a no-op, so report
freely:

```rust
#[carmy::tool(description = "Import a catalog", effect = "write")]
async fn import_catalog(ctx: AgentContext, input: Import) -> AgentResult<Imported> {
    let pages = fetch_index(&input.url).await?;
    for (i, page) in pages.iter().enumerate() {
        let items = import_page(page).await?;
        ctx.progress.report((i + 1) as f64, Some(pages.len() as f64), format!("page {}", i + 1));
        ctx.progress.partial(&items);   // a piece of the result that is already useful
    }
    Ok(Imported { pages: pages.len() })
}
```

| where | what arrives |
|-------|--------------|
| SSE | `tool.progress` events with `progress`, `total`, `message`, `partial` |
| console | `{"id":…,"event":"progress",…}` lines before the answer; the terminal UI shows them in the status line |
| MCP | `notifications/progress` for calls with a `progressToken` (no partial results), and the status message of a [task](/transports/mcp/#tasks) |
| `execute` | nothing: the plain call just waits for the result |

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
