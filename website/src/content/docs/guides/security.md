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

## Limits

- Arguments are validated against the input schema before the tool runs.
- Outputs are validated against the output schema.
- HTTP bodies are limited to 1 MiB, and `request_id`s to 256 bytes.
- The idempotency store is bounded and fails closed.
- Every execution has a [deadline](/guides/cancellation/).
