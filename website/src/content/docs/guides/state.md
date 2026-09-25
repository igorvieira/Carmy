---
title: State
description: "Inject application dependencies into tools with State<T>."
sidebar:
  order: 2
---

Register a dependency once with `.state(value)`, then ask for it by type:

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

## Rules

- **Resolved at startup.** A missing dependency fails when the app starts, never on a
  request:

  ```text
  tool registration failed: MISSING_STATE: tool `create_order` requires State<shop::Db>; register it with .state(..)
  ```

- **Keyed by type.** Register one value per type. Wrap values in newtypes (`struct
  ReadDb(Pool)`) when you need two of the same type.
- **Cloned per execution.** Every execution receives a clone, so use types that are cheap
  to clone: `Arc<T>`, connection pools and HTTP clients.
- **Any number per tool.** A tool can take several `State<T>` parameters.

```rust
#[carmy::tool(effect = "external_write")]
async fn notify(
    State(db): State<Db>,
    State(mailer): State<Arc<Mailer>>,
    input: Notification,
) -> AgentResult<()> { /* … */ Ok(()) }
```

## State is not context

`AgentContext` carries framework data about the *execution*: who is calling, which
permissions they have, and cancellation. `State<T>` carries your *application*: databases
and clients. Carmy keeps the two apart on purpose, so the context never turns into a
service locator.

The explicit alternative is to [implement `Tool`](/guides/tools/#implementing-tool-by-hand)
on a struct that holds its dependencies.
