---
title: Jobs
description: "Run tools later: a queue with retries, dead letters and schedules."
sidebar:
  order: 13
---

Some work should not happen inside a request: publishing to three channels, revalidating
a deal a day later, collecting offers every five minutes. `carmy::jobs` runs tools
later. A job is an `ExecutionRequest` with a `request_id`, so it goes through the same
runtime, with the same policies, validation, idempotency and audit as a direct call.

## Enqueue

Tools take `State<Jobs>`; the app inserts it before registering them.

```rust
use carmy::{jobs::Jobs, prelude::*, runtime::execution_request};

#[carmy::tool(description = "Run one offer through the pipeline", effect = "write")]
async fn process_offer(State(jobs): State<Jobs>, input: OfferInput) -> AgentResult<Processed> {
    let deal = build_deal(input.offer)?;
    jobs.enqueue(
        execution_request("publish_premium", json!({ "deal_id": deal.id }))
            .with_request_id(format!("publish-{}-premium", deal.id)),
    )
    .await?;
    jobs.enqueue_after(
        execution_request("publish_public", json!({ "deal_id": deal.id }))
            .with_request_id(format!("publish-{}-public", deal.id)),
        Duration::from_secs(30 * 60),
    )
    .await?;
    Ok(deal.into())
}
```

| method | effect |
|--------|--------|
| `enqueue(request)` | run as soon as a worker is free |
| `enqueue_after(request, delay)` / `enqueue_at(request, when)` | run later |
| `every(name, cron, make)` | a schedule; see below |
| `cancel(id)` | cancel a job that has not started; `true` when it did |
| `get(id)` | the job, with its status, attempts and last error |
| `dead_letters(limit)` | jobs that gave up |

**The `request_id` is the job's identity.** Enqueueing the same identity while a job
with it is queued or running returns that job's id instead of a duplicate. A collection
that runs twice, or a webhook delivered twice, never processes twice.

## Worker

```rust
carmy::app().jobs(store).run().await   // `cargo run -- worker`
```

`worker` claims due jobs and runs them, `[jobs].concurrency` at a time (default 4). Each
job holds a **lease** that the worker renews while the tool runs; a job whose lease
expires (the worker died) is claimed by another worker. On `SIGTERM` or Ctrl-C the
worker stops claiming, finishes the jobs in flight, and exits.

`Jobs::run_due(limit)` runs one round in the current task, for tests and for hosts with
their own loop.

## Retries

The error decides. See [Errors](/guides/errors/).

| outcome | what happens |
|---------|--------------|
| completed | `succeeded` |
| error with `retryable: true` | another attempt, after an exponential backoff that respects `retry_after`, up to `max_attempts` (default 5) |
| error with `retryable: false` | `failed`; no more attempts |
| `TIMEOUT`, `CANCELLED`, `TOOL_PANIC` | **uncertain**: the tool may have committed. Another attempt only if the tool is `idempotent`; otherwise `dead_lettered` with `EXECUTION_UNCERTAIN` |
| attempts exhausted | `dead_lettered` with the last error |

Each attempt runs with `request_id = "{id}#{attempt}"`, because the runtime already
recorded the previous one. A dead-lettered job is a decision for a person or an agent:
`carmy console` lists them with `dead`.

```rust
carmy::app().retry(RetryPolicy { max_attempts: 3, base: Duration::from_secs(2), cap: Duration::from_secs(300) })
```

## Schedules

```rust
carmy::app()
    .schedule("collect", "0 */5 * * * * *", || execution_request("collect_offers", json!({})))
```

Cron expressions have seven fields, seconds first. Every occurrence becomes a job with
`request_id = "collect@{timestamp}"`, so several workers ticking the same schedule
enqueue it once. A worker that was down enqueues the occurrences it missed in the last
minute, and no more.

## For agents

Every app has a `carmy_job` tool: given the `job_id` an enqueue returned, it answers the
job's status, attempts, next run, last error and, once it succeeded, the tool's
`result`. It never returns the job's arguments. While the job runs,
[`next_actions`](/transports/http/#next-actions) says when to ask again.
Because it is a tool, it works over HTTP, MCP and the console alike, and goes through
the same policies as any other.

```json
{ "tool": "carmy_job", "arguments": { "job_id": "job_4f0c…" } }
→ { "status": "completed", "data": { "job_id": "job_4f0c…", "tool": "publish_premium",
    "status": "dead_lettered", "attempts": 5, "max_attempts": 5,
    "run_at": "2026-09-27T10:00:00+00:00", "request_id": "publish-42-premium",
    "last_error": { "code": "PUBLISHER_BUSY", … } } }
```

`Carmy::operator_tools()` adds two more, for agents that operate the service:

| tool | answers |
|------|---------|
| `carmy_dead_letters` | the jobs waiting for a decision |
| `carmy_audit` | the latest executions: who, which tool, how it ended |

They reveal who ran what, so pair them with a policy such as `RequireToolPermission`.
The `carmy_` names are reserved: an app tool with one of them fails to register.

## Stores

`InMemoryJobStore` is the default: process-local, bounded, for development and tests.
In production, set a database (feature `postgres`) and the queue moves to Postgres:

```toml
[database]
url = "postgres://localhost/shop"   # or DATABASE_URL
```

Workers claim with `FOR UPDATE SKIP LOCKED`, so any number of them share a queue, and
leases follow the database clock. The store also offers a **transactional outbox**. See
[Postgres](/guides/postgres/).

Finished jobs stay until you delete them: `jobs.purge(older_than)` removes those that
succeeded, failed or were cancelled, never dead letters. When the in-memory store is
full, it drops finished jobs before refusing new ones.

Any `JobStore` implementation works; the trait has seven required methods.
