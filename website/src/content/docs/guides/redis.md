---
title: Redis
description: "Durable jobs and idempotency on Redis, shared by every instance of a service."
sidebar:
  order: 18
---

`carmy::redis` (feature `redis`) keeps the job queue and the idempotency records in
Redis. Choose it when Redis is what you already run; choose [Postgres](/guides/postgres/)
when you want the audit trail durable too, or the transactional outbox.

```toml
carmy = { version = "0.6", features = ["redis"] }
```

```toml
# carmy.toml
[database]
url = "redis://localhost:6379"   # rediss:// for TLS; CARMY_DATABASE_URL or DATABASE_URL
idempotency_retention_days = 7
```

That is all. `carmy::app()` then:

- keeps jobs and idempotency records in Redis;
- adds the `database` check to `/ready` (a `PING`);
- adds the `migrate` (nothing to do) and `cleanup` commands;
- injects the connection into tools that take `State<carmy::redis::Redis>`.

The audit trail stays in memory with Redis; the in-memory trail still feeds `carmy
console` and `carmy_audit`.

## How it works

| key | holds |
|-----|-------|
| `{carmy}:job:<id>` | a hash: the job, its status, its attempts |
| `{carmy}:queue` | queued jobs, by `run_at` |
| `{carmy}:running` | running jobs, by lease end |
| `{carmy}:dead`, `{carmy}:finished` | dead letters and finished jobs, by time |
| `{carmy}:active:<request_id>` | the active job with this identity, so enqueueing twice returns it |
| `{carmy}:idem:<identity>` | an idempotency record: fingerprint, result |

- **Atomic.** Every multi-step change (enqueue, claim, finish, cancel, reserve, complete)
  is one Lua script. Workers never share a job.
- **The Redis clock owns leases.** A claim or a heartbeat sets the lease from Redis's
  `TIME`, so workers with skewed clocks agree on when an owner is gone. Due times follow
  the app's clock.
- **One hash slot.** Every key carries the `{carmy}` hash tag, so a Redis Cluster keeps
  them together, as the scripts need.
- **Fails fast.** The connection opens on first use, reconnects on its own, and gives
  up on a call within seconds when Redis is down: `/ready` answers, and the call fails
  with a retryable `STORE_ERROR`.

## Retention

Completed idempotency records **expire** after `idempotency_retention_days` (seven by
default); in-progress records never do, since they mark executions that may have
committed. A `request_id` retried after its record expired runs again, so keep the
retention longer than any client retries.

Finished jobs stay until `cleanup` removes those older than `jobs_retention_days`
(thirty by default). Dead letters always stay.

```console
$ cargo run -- cleanup
{"jobs":5310}
```

## Piece by piece

```rust
use carmy::redis::{Redis, RedisIdempotencyStore, RedisJobStore};

let redis = Redis::open("redis://localhost")?;   // nothing connects yet
carmy::app()
    .jobs(Arc::new(RedisJobStore::new(redis.clone())))
    .idempotency_store(Arc::new(
        RedisIdempotencyStore::new(redis).with_retention(Duration::from_secs(3 * 24 * 3600)),
    ))
```
