---
title: Testes
description: "Teste tools pelo mesmo pipeline que os agentes usam."
sidebar:
  order: 8
---

O `carmy::testing` executa uma tool pelo pipeline completo do runtime: validação de
schema, policies e checagem da saída. Mantenha os testes ao lado da tool:

```rust
// src/tools/hello.rs
#[cfg(test)]
mod tests {
    use super::*;
    use carmy::serde_json::json;

    #[tokio::test]
    async fn greets_by_name() {
        let result = carmy::testing::execute(hello, json!({ "name": "Ada" })).await;
        assert_eq!(result.outcome.unwrap()["message"], "Hello, Ada!");
    }

    #[tokio::test]
    async fn rejects_empty_names() {
        let result = carmy::testing::execute(hello, json!({ "name": " " })).await;
        assert_eq!(result.outcome.unwrap_err().code, "EMPTY_NAME");
    }
}
```

## Tools com state

```rust
let app = Carmy::new().state(Db::in_memory());
let result = carmy::testing::execute_with(app, create_order, json!({ "sku": "KB-01" })).await;
```

O `execute_with` também aplica as policies do app, então dá para testar confirmação e
permissões.

## Via HTTP, sem socket

```rust
use tower::ServiceExt;

let router = Carmy::new().tool(hello).router()?;
let response = router
    .oneshot(
        axum::http::Request::post("/agent/execute")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"tool":"hello","arguments":{"name":"Ada"}}"#))?,
    )
    .await?;
assert_eq!(response.status(), 200);
```
