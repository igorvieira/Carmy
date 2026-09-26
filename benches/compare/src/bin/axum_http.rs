//! The baseline: the same tool behind a plain Axum handler, with none of Carmy's
//! guarantees (no schema validation, policies, idempotency, deadlines or tracing).
use axum::{Json, Router, routing::post};
use carmy_compare::{SearchInput, search};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Request {
    tool: String,
    arguments: Value,
}

async fn execute(Json(request): Json<Request>) -> Json<Value> {
    if request.tool != "search_products" {
        return Json(json!({ "error": "unknown tool" }));
    }
    match serde_json::from_value::<SearchInput>(request.arguments) {
        Ok(input) => Json(json!({ "status": "completed", "data": search(&input.query) })),
        Err(_) => Json(json!({ "error": "invalid arguments" })),
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let address = std::env::var("CARMY_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".into());
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(
        listener,
        Router::new().route("/agent/execute", post(execute)),
    )
    .await
}
