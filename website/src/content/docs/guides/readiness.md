---
title: Readiness and routes
description: "/health, /ready, the app's own routes and its own commands."
sidebar:
  order: 16
---

A Carmy app is a whole backend, so it answers the questions an orchestrator asks and
serves the pages a product needs, next to the agent routes and behind the same
[hardening](/guides/security/#hardening-the-http-server).

## `/health` and `/ready`

`GET /health` answers `200 {"ok": true}` while the process runs. `GET /ready` runs every
registered check together, each with a 5-second timeout, and answers `503` naming what
failed:

```json
{ "ready": false, "checks": { "database": { "ok": false, "error": { "code": "DATABASE_UNAVAILABLE", … } }, "worker": { "ok": true } } }
```

```rust
carmy::app()
    .ready("database", move || carmy::postgres::ready(pool.clone()))
    .require_worker(Duration::from_secs(60))
```

| method | checks |
|--------|--------|
| `.ready(name, check)` | any closure returning a future of `AgentResult<()>`, or a `ReadyCheck` |
| `.require_worker(within)` | a job worker ticked the queue within `within`; for servers whose jobs must actually run |

Point the load balancer's readiness probe at `/ready` and its liveness probe at
`/health`: a server whose database is gone stops receiving traffic without being
restarted.

## Own routes

```rust
let site = axum::Router::new()
    .route("/deals", get(list_deals))
    .route("/go/{id}", get(redirect));

carmy::app().routes(site)
```

The routes are merged into the same router as `/.well-known/agent`, `/agent/execute`,
the webhooks and `/ready`, so they get the same connection limits, timeouts, security
headers and CORS. Anything Axum accepts works, including your own middleware on the
merged router.

## Own commands

```rust
carmy::app()
    .command("migrate", |app| Box::pin(async move {
        let pool = connect(&std::env::var("DATABASE_URL")?).await?;
        carmy::postgres::migrate(&pool).await?;
        Ok(())
    }))
```

`cargo run -- migrate` runs it. The closure receives the builder, so it can build the
runtime or read its state. The built-in commands (`server`, `worker`, `mcp`, `console`,
`tools`) stay, and an unknown command lists every one, yours included.
