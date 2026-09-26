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

## Stores

`InMemoryJobStore` is the default: process-local, bounded, for development and tests.
In production use `carmy::postgres::PostgresJobStore` (feature `postgres`):

```rust
let pool = carmy::postgres::connect(&url).await?;
carmy::postgres::migrate(&pool).await?;
carmy::app().jobs(Arc::new(PostgresJobStore::new(pool.clone())))
```

Workers claim with `FOR UPDATE SKIP LOCKED`, so any number of them share a queue. The
store also offers a **transactional outbox**: `store.enqueue_in(&mut tx, request, run_at,
max_attempts)` inserts the job inside your transaction, so the job exists exactly when
the write it follows commits.

Any `JobStore` implementation works; the trait has seven methods and a contract test
suite you can run against your own.
