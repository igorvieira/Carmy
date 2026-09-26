---
title: Postgres
description: "Durable jobs, idempotency and audit, shared by every instance of a service."
sidebar:
  order: 17
---

The default stores live in memory: a restart loses them, and two instances of a service
cannot see each other's state. `carmy::postgres` (feature `postgres`) keeps the job
queue, the idempotency records and the audit trail in Postgres. Each piece is optional.

```toml
carmy = { version = "0.4", features = ["postgres"] }
```

```rust
use carmy::postgres::{PostgresAudit, PostgresIdempotencyStore, PostgresJobStore, connect, migrate};

let pool = connect(&std::env::var("DATABASE_URL")?).await?;   // 8 connections, 5 s acquire timeout
migrate(&pool).await?;

carmy::app()
    .jobs(Arc::new(PostgresJobStore::new(pool.clone())))
    .idempotency_store(Arc::new(PostgresIdempotencyStore::new(pool.clone())))
    .sink(Arc::new(PostgresAudit::new(pool.clone())))
    .ready("database", move || carmy::postgres::ready(pool.clone()))
```

`connect` is a convenience; any `sqlx::PgPool` works, including the one your app already
has.

## Migrations

`migrate` creates and updates the `carmy_*` tables. It is safe to run on every start and
from several instances at once: they take turns on an advisory lock. Carmy records its
versions in its own `carmy_schema_version` table and never touches `_sqlx_migrations`,
so it lives next to your application's migrations without conflict.

| table | holds |
|-------|-------|
| `carmy_jobs` | the job queue |
| `carmy_idempotency` | one row per request identity, with its fingerprint and result |
| `carmy_audit` | one row per execution, never arguments or outputs |
| `carmy_schema_version` | which Carmy migrations ran |

## Jobs

- **Enqueueing never duplicates.** A partial unique index on `request_id` covers queued
  and running jobs; enqueueing an identity that is already active returns that job.
- **Workers share one queue.** Claims use `SELECT … FOR UPDATE SKIP LOCKED`: each
  worker skips rows another one holds, so no job runs twice and no worker waits.
- **Leases use the database clock.** A claimed job is owned until `now() + lease` on the
  database, renewed while the tool runs. Workers on machines with skewed clocks still
  agree on when an owner is gone. Due times (`run_at`) follow the app's clock.
- **The transactional outbox** enqueues inside your own transaction, so the job exists
  exactly when the write it follows commits:

```rust
let mut tx = pool.begin().await?;
sqlx::query("INSERT INTO orders …").execute(&mut *tx).await?;
store.enqueue_in(&mut tx, request, chrono::Utc::now(), 5).await?;
tx.commit().await?;   // the order and its job exist together, or neither does
```

The outbox takes a `sqlx` transaction. An app on another database library can still use
every store, but not the outbox.

## Idempotency

Reservations are an `INSERT … ON CONFLICT DO NOTHING`, so two instances receiving the
same `request_id` at once agree on one owner. The semantics are the in-memory store's:
replay, `IDEMPOTENCY_CONFLICT`, `EXECUTION_UNCERTAIN`. See [Idempotency](/guides/idempotency/).

## Audit

Executions never wait on the database: records go through a bounded channel to one
writer task. When the channel is full, a record is dropped and a warning logged. See
[Audit](/guides/audit/).

## Retention

Nothing is deleted on its own. `cleanup` removes what a `Retention` no longer keeps, in
batches of 5000 rows, with ages on the database clock:

| kind | default | always kept |
|------|---------|-------------|
| finished jobs (succeeded, failed, cancelled) | 30 days | queued, running and dead-lettered jobs |
| completed idempotency records | 7 days | in-progress records, which mark uncertain executions |
| audit records | 90 days | |

A `request_id` retried after its idempotency record is gone **runs again**, so keep
that retention longer than any client retries.

Run it from a command, and schedule the command with cron, a Kubernetes CronJob or any
scheduler:

```rust
carmy::app().command("cleanup", move |_| Box::pin(async move {
    let cleaned = carmy::postgres::cleanup(&pool, Retention::default()).await?;
    println!("{cleaned:?}");
    Ok(())
}))
```

```console
$ cargo run -- cleanup
{"audit":1204,"idempotency":88,"jobs":5310}
```

`Jobs::purge(older_than)` does the job part through any `JobStore`, the in-memory one
included.

## Readiness

`carmy::postgres::ready(pool)` runs `SELECT 1`. Behind `/ready`, it takes an instance out
of the load balancer when its database is unreachable. See
[Readiness and routes](/guides/readiness/).
