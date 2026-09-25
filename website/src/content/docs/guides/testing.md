---
title: Testing
description: "Test tools through the same pipeline agents use."
sidebar:
  order: 8
---

`carmy::testing` executes a tool through the full runtime pipeline: schema validation,
policies and output checks. Keep the tests next to the tool:

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

## Tools with state

```rust
let app = Carmy::new().state(Db::in_memory());
let result = carmy::testing::execute_with(app, create_order, json!({ "sku": "KB-01" })).await;
```

`execute_with` also applies the app's policies, so you can test confirmation and
permissions too.

## Over HTTP, without a socket

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
