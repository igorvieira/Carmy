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

## Rate limiting

`RateLimit` é uma política pronta: uma janela fixa por principal (ou por sessão, e senão
uma janela anônima compartilhada). As rejeições são `RATE_LIMITED`, na categoria
`capacity`, com retry permitido e `retry_after` em segundos:

```rust
use carmy::runtime::RateLimit;
use std::time::Duration;

carmy::app()
    .policy(RateLimit::new(60, Duration::from_secs(60)))
    .run()
    .await
```

Ela é local ao processo. Com várias instâncias, coloque um limitador compartilhado na
frente (um gateway, ou um middleware Tower com uma store).

## Limites

- Os argumentos são validados contra o schema de entrada antes de a tool rodar.
- As saídas são validadas contra o schema de saída.
- O corpo HTTP é limitado a 1 MiB, e o `request_id` a 256 bytes.
- O store de idempotência é limitado e falha de forma segura.
- Toda execução tem um [prazo](/pt-br/guides/cancellation/).

## Endurecendo o servidor HTTP

O prazo da execução começa quando a tool começa. Tudo antes disso é limitado pelo
`ServerOptions`, então um servidor Carmy pode ficar exposto à internet sem um proxy na
frente:

| proteção | padrão | chave `[http]` no `carmy.toml` |
|----------|--------|--------------------------------|
| timeout de cabeçalhos, também para conexões keep-alive ociosas | 10 s | `header_timeout_secs` |
| timeout do corpo | 30 s | `body_timeout_secs` |
| conexões simultâneas; as demais esperam no backlog do accept | 4096 | `max_connections` |
| cabeçalhos de segurança (`nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy`, uma CSP que nega tudo) | desligado | `security_headers` |
| CORS | desligado: nenhuma origem de navegador pode chamar a API | só em código |

Os cabeçalhos e o CORS vêm desligados porque a API serve JSON para agentes, não páginas
para navegadores. Ligue-os quando um navegador for chamá-la:

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

Cada proteção tem um teste de contrato sobre um socket real: um cliente que manda os
cabeçalhos aos poucos, um que nunca manda o corpo, uma conexão keep-alive ociosa e
conexões além do limite. Rode-os com `cargo test -p carmy-http --test hardening`.

O `Strict-Transport-Security` não é definido, porque o Carmy não termina TLS. Defina-o
no proxy ou no balanceador que termina.

## O que outros frameworks checam, e a resposta do Carmy

| check | Express | Rails | FastAPI | Carmy |
|-------|---------|-------|---------|-------|
| cabeçalhos de segurança | `helmet` | padrão | não | `security_headers` (opt-in) |
| CORS | `cors` | gem | embutido | `ServerOptions::cors` (opt-in) |
| rate limiting | pacote | `rack-attack` | pacote | política `RateLimit` |
| CSRF | `csurf` | padrão | não | não se aplica: só JSON, sem cookies |
| timeouts para clientes lentos | proxy | servidor | uvicorn | embutidos e testados |
| limite de corpo | sim | sim | sim | 1 MiB |
| auditoria de dependências | `npm audit` | `bundler-audit` | `pip-audit` | `cargo deny` no CI |
| fuzzing | raro | raro | raro | `cargo fuzz` no CI, no protocolo do console e no `/agent/execute` |
| código `unsafe` | n/a | n/a | n/a | nenhum nas crates do Carmy |

O que o Carmy checa e esses frameworks não: efeitos declarados, confirmação confiável
para tools destrutivas, contexto que a requisição não consegue forjar, validação de
entrada *e* saída, retries seguros para replay, e logs sem payloads.

Nenhuma auditoria de segurança independente do Carmy foi feita. Reporte vulnerabilidades
em privado pelo [GitHub](https://github.com/igorvieira/Carmy/security/advisories/new).
