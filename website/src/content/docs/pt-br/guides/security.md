---
title: Segurança
description: "Autenticação, autorização, policies e limites."
sidebar:
  order: 10
---

O Carmy oferece hooks pequenos em vez de um framework de políticas.

## Autenticação: o contexto confiável

O corpo da requisição nunca define o contexto. Quem define é o host, depois da
autenticação. Via HTTP, insira um `AgentContext` a partir de um middleware:

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

Via MCP, defina o contexto da conexão com `McpServer::context(ctx)`.

<small>Se os nomes e schemas das tools forem privados, coloque a autenticação na frente do
router inteiro, incluindo a descoberta.</small>

## Autorização: policies

Uma `ExecutionPolicy` roda antes de toda execução, inclusive nos resultados reenviados:

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

Policies embutidas:

- **`SafePolicy`** está sempre ativa. Ela exige `confirm:<tool>` para tools destrutivas e
  para tools que exigem confirmação.
- **`RequireToolPermission`** é opcional. Ela exige `tool:<name>` em toda chamada.

Rate limiting cabe no mesmo hook (retorne `ErrorCategory::Capacity` com `.retryable(…)`),
ou pode ficar num middleware Tower no router.

## Limites

- Os argumentos são validados contra o schema de entrada antes de a tool rodar.
- As saídas são validadas contra o schema de saída.
- O corpo HTTP é limitado a 1 MiB, e o `request_id` a 256 bytes.
- O store de idempotência é limitado e falha de forma segura.
- Toda execução tem um [prazo](/pt-br/guides/cancellation/).
