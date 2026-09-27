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

Then give it a URL, like any other setting:

```toml
# carmy.toml
[database]
url = "postgres://localhost/shop"   # CARMY_DATABASE_URL, then DATABASE_URL, win over this
```

That is all. `carmy::app()` then:

- keeps jobs, idempotency records and the audit trail in Postgres;
- migrates Carmy's tables before any command but `tools`;
- adds the `database` check to `/ready`;
- adds the `migrate` and `cleanup` commands;
- injects the pool into tools that take `State<PgPool>`, and the job store into those
  that take `State<PostgresJobStore>` (for the outbox).

```rust
use carmy::postgres::sqlx::PgPool;

#[carmy::tool(description = "Create an order", effect = "write")]
async fn create_order(State(pool): State<PgPool>, input: NewOrder) -> AgentResult<Order> {
    sqlx::query_as("INSERT INTO orders (sku) VALUES ($1) RETURNING id, sku")
        .bind(input.sku)
        .fetch_one(&pool)
        .await
        .map_err(|e| AgentError::new("DB", e.to_string(), ErrorCategory::Internal).retryable(Some(1)))
}
```

The pool connects on first use, so a database that is down at start makes `/ready` fail
rather than the process. In code, `Carmy::database(url)` does the same as the setting.

### Piece by piece

Each store also works alone, on any `sqlx::PgPool`:

```rust
use carmy::postgres::{PostgresAudit, PostgresIdempotencyStore, PostgresJobStore, connect, migrate};

let pool = connect(&url).await?;   // 8 connections, 5 s acquire timeout
migrate(&pool).await?;
carmy::app()
    .jobs(Arc::new(PostgresJobStore::new(pool.clone())))
    .idempotency_store(Arc::new(PostgresIdempotencyStore::new(pool.clone())))
    .sink(Arc::new(PostgresAudit::new(pool.clone())))
```

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

With a database configured, the `cleanup` command runs it. Schedule the command with
cron, a Kubernetes CronJob or any scheduler:

```console
$ cargo run -- cleanup
{"audit":1204,"idempotency":88,"jobs":5310}
```

```toml
[database]
jobs_retention_days = 30
idempotency_retention_days = 7
audit_retention_days = 90
```

In code: `.retention(Retention { .. })`, or `carmy::postgres::cleanup(&pool, retention)`
anywhere.

`Jobs::purge(older_than)` does the job part through any `JobStore`, the in-memory one
included.

## Readiness

`carmy::postgres::ready(pool)` runs `SELECT 1`. Behind `/ready`, it takes an instance out
of the load balancer when its database is unreachable. See
[Readiness and routes](/guides/readiness/).
