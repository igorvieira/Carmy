---
title: Security
description: "Authentication, authorization, policies and limits."
sidebar:
  order: 10
---

Carmy provides small hooks rather than a policy framework.

## Authentication: the trusted context

Request bodies can never set the context. The host sets it after authentication. Over
HTTP, insert an `AgentContext` from middleware:

```rust
use axum::{extract::Request, middleware::{self, Next}, response::Response};
use carmy::AgentContext;

async fn authenticate(mut request: Request, next: Next) -> Response {
    let mut ctx = AgentContext::default();
    if let Some(user) = verify_token(request.headers()) {
        ctx.principal = Some(user.id);
        if user.is_admin {
            ctx.permissions.insert("confirm:delete_customer".into());
        }
    }
    request.extensions_mut().insert(ctx);
    next.run(request).await
}

#[tokio::main]
async fn main() -> carmy::Result {
    let router = carmy::app().router()?.layer(middleware::from_fn(authenticate));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, router).await?;
    Ok(())
}
```

Over MCP, set the context of the connection with `McpServer::context(ctx)`.

<small>If tool names and schemas are private, put authentication in front of the whole
router, discovery included.</small>

## Authorization: policies

An `ExecutionPolicy` runs before every execution, including replays:

```rust
use carmy::{runtime::ExecutionPolicy, ToolMetadata};

struct AdminOnlyDestructive;

impl ExecutionPolicy for AdminOnlyDestructive {
    fn check(&self, ctx: &AgentContext, tool: &ToolMetadata) -> AgentResult<()> {
        if tool.effect == Effect::Destructive && !ctx.permissions.contains("admin") {
            return Err(AgentError::new("ADMIN_ONLY", "Admins only", ErrorCategory::Permission));
        }
        Ok(())
    }
}

carmy::app().policy(AdminOnlyDestructive).run().await
```

Built-in policies:

- **`SafePolicy`** is always on. It requires `confirm:<tool>` for destructive tools and
  tools that require confirmation.
- **`RequireToolPermission`** is opt-in. It requires `tool:<name>` for every call.

Rate limiting fits the same hook (return `ErrorCategory::Capacity` with `.retryable(…)`),
or can live in Tower middleware on the router.

## Rate limiting

`RateLimit` is a ready-made policy: a fixed window per principal (or per session, then a
shared anonymous window). Rejections are `RATE_LIMITED`, in the `capacity` category,
retryable, with `retry_after` in seconds:

```rust
use carmy::runtime::RateLimit;
use std::time::Duration;

carmy::app()
    .policy(RateLimit::new(60, Duration::from_secs(60)))
    .run()
    .await
```

It is process-local. Put a shared limiter (a gateway, or Tower middleware backed by a
store) in front of several instances.

## Limits

- Arguments are validated against the input schema before the tool runs.
- Outputs are validated against the output schema.
- HTTP bodies are limited to 1 MiB, and `request_id`s to 256 bytes.
- The idempotency store is bounded and fails closed.
- Every execution has a [deadline](/guides/cancellation/).

## Hardening the HTTP server

The execution deadline starts when a tool starts. Everything before that is bounded by
`ServerOptions`, so a Carmy server can face the internet without a proxy in front:

| protection | default | `carmy.toml` `[http]` key |
|------------|---------|---------------------------|
| header timeout, also for idle keep-alive connections | 10 s | `header_timeout_secs` |
| body timeout | 30 s | `body_timeout_secs` |
| concurrent connections; extra ones wait in the accept backlog | 4096 | `max_connections` |
| security headers (`nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy`, a deny-all CSP) | off | `security_headers` |
| CORS | off: no browser origin may call the API | code only |

The headers and CORS are off because the API serves JSON to agents, not pages to
browsers. Turn them on when a browser will call it:

```rust
use carmy::http::{Any, CorsLayer, ServerOptions};

carmy::app()
    .http(ServerOptions {
        security_headers: true,
        cors: Some(CorsLayer::new().allow_origin(["https://app.example".parse().unwrap()])),
        ..ServerOptions::default()
    })
    .run()
    .await
```

Every protection has a contract test over a real socket: a client that trickles headers,
one that never sends its body, an idle keep-alive connection, and connections beyond the
limit. Run them with `cargo test -p carmy-http --test hardening`.

`Strict-Transport-Security` is not set, because Carmy does not terminate TLS. Set it at
the proxy or load balancer that does.

## What other frameworks check, and Carmy's answer

| check | Express | Rails | FastAPI | Carmy |
|-------|---------|-------|---------|-------|
| security headers | `helmet` | default | no | `security_headers` (opt-in) |
| CORS | `cors` | gem | built in | `ServerOptions::cors` (opt-in) |
| rate limiting | package | `rack-attack` | package | `RateLimit` policy |
| CSRF | `csurf` | default | no | not applicable: JSON only, no cookies |
| slow-client timeouts | proxy | server | uvicorn | built in, tested |
| body limit | yes | yes | yes | 1 MiB |
| dependency audit | `npm audit` | `bundler-audit` | `pip-audit` | `cargo deny` in CI |
| fuzzing | rare | rare | rare | `cargo fuzz` in CI, on the console protocol and `/agent/execute` |
| `unsafe` code | n/a | n/a | n/a | none in Carmy's crates |

What Carmy checks that these frameworks don't: declared effects, trusted confirmation
for destructive tools, context that requests cannot forge, input *and* output
validation, replay-safe retries, and logs without payloads.

No independent security audit of Carmy has been done. Report vulnerabilities privately
through [GitHub](https://github.com/igorvieira/Carmy/security/advisories/new).
