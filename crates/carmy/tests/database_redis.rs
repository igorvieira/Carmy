//! A Redis database from a URL. Needs `CARMY_TEST_REDIS_URL`; skips otherwise.
#![cfg(feature = "redis")]
use axum::{body::Body, http::Request};
use carmy::prelude::*;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

static CALLS: AtomicUsize = AtomicUsize::new(0);

#[derive(Deserialize, JsonSchema)]
struct Note {
    text: String,
}
#[carmy::tool(description = "Record a note", effect = "write", register = false)]
async fn note(State(_redis): State<carmy::redis::Redis>, input: Note) -> AgentResult<String> {
    CALLS.fetch_add(1, Ordering::SeqCst);
    Ok(input.text)
}

async fn send(app: &axum::Router, request: Request<Body>) -> (u16, Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_redis_url_wires_the_stores_and_the_commands() {
    let Ok(url) = std::env::var("CARMY_TEST_REDIS_URL") else {
        eprintln!("CARMY_TEST_REDIS_URL is unset; skipping");
        return;
    };
    let app = || Carmy::new().database(&url).tool(note);
    app().run_command(Some("migrate")).await.unwrap();
    app().run_command(Some("cleanup")).await.unwrap();

    let request_id = format!("redis-{}", std::process::id());
    let execute = || {
        Request::post("/agent/execute")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"tool": "note", "arguments": {"text": "hi"}, "request_id": request_id})
                    .to_string(),
            ))
            .unwrap()
    };
    let (status, first) = send(&app().router().unwrap(), execute()).await;
    assert_eq!(status, 200, "{first}");
    // Another instance replays from Redis: the tool ran once.
    let (_, second) = send(&app().router().unwrap(), execute()).await;
    assert_eq!(second["execution_id"], first["execution_id"]);
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);

    let (status, ready) = send(
        &app().router().unwrap(),
        Request::get("/ready").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, 200, "{ready}");
    assert_eq!(ready["checks"]["database"]["ok"], true);
}

#[test]
fn unknown_schemes_are_configuration_errors() {
    let error = Carmy::new()
        .database("mysql://localhost/shop")
        .build()
        .err()
        .expect("a configuration error");
    assert!(
        error.to_string().contains("postgres:// or redis://"),
        "{error}"
    );
}
