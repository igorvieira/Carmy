---
title: State
description: "Injete dependências da aplicação nas tools com State<T>."
sidebar:
  order: 2
---

Registre uma dependência uma vez com `.state(value)` e peça por ela pelo tipo:

```rust
use carmy::prelude::*;

#[derive(Clone)]
struct Db(sqlx::PgPool);

#[carmy::tool(description = "Place an order", effect = "write")]
async fn create_order(State(db): State<Db>, input: NewOrder) -> AgentResult<Order> {
    db.insert(input).await
}

#[tokio::main]
async fn main() -> carmy::Result {
    let db = Db(sqlx::PgPool::connect("postgres://localhost/shop").await.expect("database"));
    carmy::app().state(db).run().await
}
```

## Regras

- **Resolvido na inicialização.** Uma dependência ausente falha quando o app sobe, nunca
  em uma requisição:

  ```text
  tool registration failed: MISSING_STATE: tool `create_order` requires State<shop::Db>; register it with .state(..)
  ```

- **Indexado por tipo.** Registre um valor por tipo. Use newtypes (`struct ReadDb(Pool)`)
  quando precisar de dois valores do mesmo tipo.
- **Clonado a cada execução.** Toda execução recebe um clone, então use tipos baratos de
  clonar: `Arc<T>`, pools de conexão e clientes HTTP.
- **Quantos quiser por tool.** Uma tool pode receber vários parâmetros `State<T>`.

```rust
#[carmy::tool(effect = "external_write")]
async fn notify(
    State(db): State<Db>,
    State(mailer): State<Arc<Mailer>>,
    input: Notification,
) -> AgentResult<()> { /* … */ Ok(()) }
```

## State não é contexto

O `AgentContext` carrega dados do framework sobre a *execução*: quem está chamando, quais
permissões tem e o cancelamento. O `State<T>` carrega a sua *aplicação*: bancos de dados e
clientes. O Carmy mantém os dois separados de propósito, para que o contexto nunca vire um
service locator.

A alternativa explícita é [implementar `Tool`](/pt-br/guides/tools/#implementando-tool-à-mão)
em uma struct que guarda as próprias dependências.
