---
title: Cancellation and timeouts
description: "Deadlines and cancellation that reach your tool code."
sidebar:
  order: 7
---

Agents abandon executions. Carmy treats that as a first-class event, not an accident.

## Deadlines

Every execution has a deadline: 30 seconds by default.

```toml
# carmy.toml
timeout_secs = 10    # or CARMY_TIMEOUT_SECS=10
```

```rust
carmy::app().timeout(std::time::Duration::from_secs(10)).run().await
```

A timed-out execution returns `TIMEOUT` (status `timed_out`, HTTP 504).

## The cancellation token

Every execution has a `CancellationToken` in `ctx.cancellation`. The runtime cancels it
and stops polling the tool when any of these happens:

- the deadline passes
- the HTTP client disconnects, on a JSON or SSE request
- an MCP client sends `notifications/cancelled`
- the caller drops the execution future or stream

Work that a tool spawns in the background should watch the token:

```rust
#[carmy::tool(effect = "external_write")]
async fn export_report(ctx: AgentContext, input: ExportInput) -> AgentResult<Export> {
    let token = ctx.cancellation.clone();
    let upload = tokio::spawn(async move {
        tokio::select! {
            _ = token.cancelled() => Err("cancelled"),
            result = upload_to_storage(input) => result,
        }
    });
    // …
}
```

## Semantics

Cancellation cannot un-send an email. That's why Carmy records an interrupted execution
as **uncertain**: its idempotency reservation is kept, and a retry reports
`EXECUTION_UNCERTAIN` instead of running the side effect twice. See
[Idempotency](/guides/idempotency/).
