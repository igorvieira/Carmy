---
title: Readiness e rotas
description: "/health, /ready, as rotas próprias do app e os comandos próprios."
sidebar:
  order: 16
---

Um app Carmy é um backend inteiro, então ele responde às perguntas que um orquestrador
faz e serve as páginas de que um produto precisa, ao lado das rotas de agente e atrás do
mesmo [endurecimento](/pt-br/guides/security/#endurecendo-o-servidor-http).

## `/health` e `/ready`

`GET /health` responde `200 {"ok": true}` enquanto o processo roda. `GET /ready` roda
todos os checks registrados juntos, cada um com timeout de 5 segundos, e responde `503`
nomeando o que falhou:

```json
{ "ready": false, "checks": { "database": { "ok": false, "error": { "code": "DATABASE_UNAVAILABLE", … } }, "worker": { "ok": true } } }
```

```rust
carmy::app()
    .ready("database", move || carmy::postgres::ready(pool.clone()))
    .require_worker(Duration::from_secs(60))
```

| método | verifica |
|--------|----------|
| `.ready(name, check)` | qualquer closure que devolva um future de `AgentResult<()>`, ou um `ReadyCheck` |
| `.require_worker(within)` | um worker de jobs marcou a fila dentro de `within`; para servidores cujos jobs precisam de fato rodar |

Aponte a sonda de readiness do load balancer para `/ready` e a de liveness para
`/health`: um servidor cujo banco sumiu para de receber tráfego sem ser reiniciado.

## Rotas próprias

```rust
let site = axum::Router::new()
    .route("/deals", get(list_deals))
    .route("/go/{id}", get(redirect));

carmy::app().routes(site)
```

As rotas entram no mesmo router de `/.well-known/agent`, `/agent/execute`, dos webhooks e
de `/ready`, então recebem os mesmos limites de conexão, timeouts, headers de segurança e
CORS. Tudo o que o Axum aceita funciona, inclusive middleware seu no router combinado.

## Comandos próprios

```rust
carmy::app()
    .command("migrate", |app| Box::pin(async move {
        let pool = connect(&std::env::var("DATABASE_URL")?).await?;
        carmy::postgres::migrate(&pool).await?;
        Ok(())
    }))
```

`cargo run -- migrate` o executa. A closure recebe o builder, então pode construir o
runtime ou ler o estado dele. Os comandos embutidos (`server`, `worker`, `mcp`,
`console`, `tools`) continuam, e um comando desconhecido lista todos, os seus incluídos.
