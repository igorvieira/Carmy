use axum::{body::Body, http::Request, routing::get};
use carmy::prelude::*;
use http_body_util::BodyExt;
use serde_json::Value;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tower::ServiceExt;

#[carmy::tool(description = "Say hi", effect = "none", register = false)]
async fn hi() -> AgentResult<String> {
    Ok("hi".into())
}

async fn json_body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn an_app_serves_its_own_routes_and_readiness_next_to_the_agent_routes() {
    let db_up = Arc::new(AtomicBool::new(false));
    let up = db_up.clone();
    let app = Carmy::new()
        .tool(hi)
        .routes(axum::Router::new().route("/go/{id}", get(|| async { "redirect" })))
        .ready("database", move || {
            let ok = up.load(Ordering::SeqCst);
            async move {
                if ok {
                    Ok(())
                } else {
                    Err(AgentError::new("DB_DOWN", "down", ErrorCategory::Capacity))
                }
            }
        })
        .require_worker(Duration::from_secs(30))
        .router()
        .unwrap();

    let own = app
        .clone()
        .oneshot(Request::get("/go/42").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(own.status(), 200);
    let agent = app
        .clone()
        .oneshot(
            Request::get("/.well-known/agent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(agent.status(), 200);

    let not_ready = app
        .clone()
        .oneshot(Request::get("/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(not_ready.status(), 503);
    let body = json_body(not_ready).await;
    assert_eq!(body["checks"]["database"]["error"]["code"], "DB_DOWN");
    assert_eq!(
        body["checks"]["worker"]["ok"], false,
        "no worker has ticked yet"
    );

    db_up.store(true, Ordering::SeqCst);
    let still = app
        .clone()
        .oneshot(Request::get("/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(still.status(), 503, "the worker check still fails");
    let health = app
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(health.status(), 200);
}

#[tokio::test]
async fn app_commands_run_with_the_built_app() {
    let ran = Arc::new(AtomicBool::new(false));
    let flag = ran.clone();
    let app = Carmy::new().tool(hi).command("migrate", move |app| {
        let flag = flag.clone();
        Box::pin(async move {
            let runtime = app.build()?;
            assert_eq!(runtime.tools().len(), 1);
            flag.store(true, Ordering::SeqCst);
            Ok(())
        })
    });
    app.run_command(Some("migrate")).await.unwrap();
    assert!(ran.load(Ordering::SeqCst));

    let unknown = Carmy::new().tool(hi).run_command(Some("nope")).await;
    let message = unknown.err().unwrap().to_string();
    assert!(message.contains("unknown command `nope`"), "{message}");
    assert!(message.contains("server"), "{message}");
}
