---
title: Audit
description: "Who ran what, when, and how it ended, without arguments or outputs."
sidebar:
  order: 15
---

Every execution leaves an `ExecutionRecord`: replays, policy rejections and validation
failures included. Records never carry arguments or outputs, which may hold secrets, so
the trail is safe to keep and to show.

```json
{
  "execution_id": "exec_d8df…",
  "request_id": "publish-kb-01-premium",
  "principal": "worker",
  "session": null,
  "tool": "publish_premium",
  "effect": "external_write",
  "status": "completed",
  "error_code": null,
  "duration_ms": 12,
  "started_at": "2026-09-26T14:03:11.204Z",
  "replayed": false
}
```

## Sinks

An `ExecutionSink` receives each record on the execution's task, so it must return at
once. The runtime accepts any number:

```rust
carmy::app()
    .sink(Arc::new(PostgresAudit::new(pool)))   // durable; feature `postgres`
```

| sink | keeps |
|------|-------|
| `InMemoryAudit` | the last 1000 records; every app has one, and `carmy console` reads it |
| `carmy::postgres::PostgresAudit` | the `carmy_audit` table, written by one task behind a bounded channel; under backpressure it drops a record and logs a warning rather than slowing executions |

Your own sink is a trait with one method: `fn record(&self, record: ExecutionRecord)`.

## Reading the trail

In `carmy console`, or over `carmy-console/1`:

```text
audit 20            -> the latest records, newest first
dead                -> jobs in the dead-letter queue
```

`PostgresAudit::recent(limit)` and `InMemoryAudit::recent(limit)` return the same
records in code. A replayed execution has `replayed: true` and the **original**
`execution_id`, so a redelivered webhook shows up as two records pointing at one
execution.
