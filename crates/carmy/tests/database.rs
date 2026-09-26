//! A database from configuration. Needs `CARMY_TEST_DATABASE_URL`; skips otherwise.
#![cfg(feature = "postgres")]
use axum::{body::Body, http::Request};
use carmy::postgres::sqlx::{self, PgPool};
use carmy::prelude::*;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

fn url() -> Option<String> {
    let url = std::env::var("CARMY_TEST_DATABASE_URL").ok();
    if url.is_none() {
        eprintln!("CARMY_TEST_DATABASE_URL is unset; skipping");
    }
    url
}

#[derive(Deserialize, JsonSchema)]
struct Note {
    text: String,
}
#[carmy::tool(
    description = "Echo through the database",
    effect = "write",
    register = false
)]
async fn echo(State(pool): State<PgPool>, input: Note) -> AgentResult<String> {
    let text: String = sqlx::query_scalar("SELECT $1::text")
        .bind(input.text)
        .fetch_one(&pool)
        .await
        .map_err(|e| AgentError::new("DB", e.to_string(), ErrorCategory::Internal))?;
    Ok(text)
}

async fn send(app: &axum::Router, request: Request<Body>) -> (u16, Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_database_url_wires_every_store_and_the_commands() {
    let Some(url) = url() else { return };
    let app = || Carmy::new().database(&url).tool(echo);

    // `migrate` and `cleanup` exist once a database is configured.
    app().run_command(Some("migrate")).await.unwrap();
    app().run_command(Some("cleanup")).await.unwrap();

    let router = app().router().unwrap();
    let request_id = format!("db-{}", std::process::id());
    let execute = || {
        Request::post("/agent/execute")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"tool": "echo", "arguments": {"text": "hi"}, "request_id": request_id})
                    .to_string(),
            ))
            .unwrap()
    };
    let (status, first) = send(&router, execute()).await;
    assert_eq!(status, 200, "{first}");
    assert_eq!(first["data"], "hi", "State<PgPool> reaches the tool");

    // Another instance of the same app replays from Postgres.
    let other = app().router().unwrap();
    let (_, second) = send(&other, execute()).await;
    assert_eq!(second["execution_id"], first["execution_id"]);

    let (status, ready) = send(&router, Request::get("/ready").body(Body::empty()).unwrap()).await;
    assert_eq!(status, 200, "{ready}");
    assert_eq!(ready["checks"]["database"]["ok"], true);

    // Jobs land in carmy_jobs.
    let (_, jobs) = app().router_and_jobs().unwrap();
    let id = jobs
        .enqueue(carmy::runtime::execution_request(
            "echo",
            json!({"text": "later"}),
        ))
        .await
        .unwrap();
    let pool = carmy::postgres::connect(&url).await.unwrap();
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM carmy_jobs WHERE id = $1")
        .bind(&id.0)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 1);
}

#[test]
fn bad_urls_and_retention_without_a_database_are_configuration_errors() {
    let bad = Carmy::new()
        .database("not a url")
        .build()
        .err()
        .expect("a configuration error");
    assert!(bad.to_string().contains("database url"), "{bad}");
    let orphan = Carmy::new()
        .retention(carmy::postgres::Retention::default())
        .build()
        .err()
        .expect("a configuration error");
    assert!(orphan.to_string().contains("needs a database"), "{orphan}");
}
