---
title: Configuration
description: "carmy.toml, environment variables, commands and features."
sidebar:
  order: 9
---

`carmy::app()` reads `carmy.toml` from the working directory, then applies environment
variables. The precedence is environment, then file, then defaults.

| key | env var | default |
|-----|---------|---------|
| `name` | `CARMY_NAME` | `carmy` |
| `address` | `CARMY_ADDR` | `127.0.0.1:3000` |
| `timeout_secs` | `CARMY_TIMEOUT_SECS` | `30` |

The `[http]` table holds the server protections (see
[Security](/guides/security/#hardening-the-http-server)):

| key | env var | default |
|-----|---------|---------|
| `http.header_timeout_secs` | `CARMY_HTTP_HEADER_TIMEOUT_SECS` | `10` |
| `http.body_timeout_secs` | `CARMY_HTTP_BODY_TIMEOUT_SECS` | `30` |
| `http.max_connections` | `CARMY_HTTP_MAX_CONNECTIONS` | `4096` |
| `http.security_headers` | `CARMY_HTTP_SECURITY_HEADERS` | `false` |

The `[jobs]` table configures the worker (see [Jobs](/guides/jobs/)):

| key | env var | default |
|-----|---------|---------|
| `jobs.concurrency` | `CARMY_JOBS_CONCURRENCY` | `4` |
| `jobs.max_attempts` | `CARMY_JOBS_MAX_ATTEMPTS` | `5` |

`CARMY_CONFIG=path/to/file.toml` reads another file. Unknown keys are errors, so typos
don't go unnoticed:

```text
invalid configuration: carmy.toml: unknown field `adress`, expected one of `name`, `address`, `timeout_secs`
```

## Commands

`run()` chooses what to do from the first command-line argument:

| command | effect |
|---------|--------|
| *(none)* or `server` | serve HTTP on `address` |
| `mcp` | serve MCP over stdin/stdout |
| `tools` | print the tool catalog as JSON and exit |
| `console` | serve `carmy-console/1` on stdio (see [Console](/guides/console/)) |
| `worker` | run the jobs and the schedules (see [Jobs](/guides/jobs/)) |
| *(yours)* | anything added with `.command(..)` (see [Readiness and routes](/guides/readiness/#own-commands)) |

## Builder

Everything in the file can also be set in code, and code wins:

```rust
carmy::app()
    .name("shop")
    .address("0.0.0.0:8080")
    .timeout(std::time::Duration::from_secs(10))
    .state(db)
    .policy(RequireToolPermission)
    .jobs(Arc::new(PostgresJobStore::new(pool.clone())))
    .schedule("collect", "0 */5 * * * * *", || execution_request("collect_offers", json!({})))
    .webhook("/webhooks/billing", Webhook::to("billing_event").verify(verify::hmac_sha256(secret, "X-Signature")).event_id("/id").enqueue())
    .ready("database", move || carmy::postgres::ready(pool.clone()))
    .routes(site)
    .run()
    .await
```

`Carmy::new()` ignores the file, the environment and auto-registration entirely.

## Features

| feature | default | provides |
|---------|---------|----------|
| `http` | yes | `carmy::http`, `.router()`, `.listen()` |
| `mcp` | yes | `carmy::mcp`, `.serve_mcp_stdio()` |
| `observability` | yes | the tracing subscriber installed by `run()` |
| `postgres` | no | `carmy::postgres`: durable job, idempotency and audit stores |

```toml
carmy = { version = "0.4", default-features = false, features = ["http"] }
```
