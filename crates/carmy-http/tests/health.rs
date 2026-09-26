use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use carmy_core::{AgentError, ErrorCategory};
use carmy_http::{ReadyCheck, health_router};
use http_body_util::BodyExt;
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tower::ServiceExt;

async fn json_body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn ready_reports_every_check_and_health_only_the_process() {
    let db_up = Arc::new(AtomicBool::new(true));
    let db = {
        let up = db_up.clone();
        move || {
            let up = up.load(Ordering::SeqCst);
            async move {
                if up {
                    Ok(())
                } else {
                    Err(AgentError::new(
                        "DB_DOWN",
                        "connection refused",
                        ErrorCategory::Capacity,
                    ))
                }
            }
        }
    };
    let queue = || async { Ok(()) };
    let checks: Vec<(String, Arc<dyn ReadyCheck>)> = vec![
        ("database".into(), Arc::new(db)),
        ("queue".into(), Arc::new(queue)),
    ];
    let app = health_router(checks);

    let ok = app
        .clone()
        .oneshot(Request::get("/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let body = json_body(ok).await;
    assert_eq!(body["ready"], true);
    assert_eq!(body["checks"]["database"]["ok"], true);
    assert_eq!(body["checks"]["queue"]["ok"], true);

    db_up.store(false, Ordering::SeqCst);
    let down = app
        .clone()
        .oneshot(Request::get("/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(down.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = json_body(down).await;
    assert_eq!(body["ready"], false);
    assert_eq!(body["checks"]["database"]["ok"], false);
    assert_eq!(body["checks"]["database"]["error"]["code"], "DB_DOWN");
    assert_eq!(body["checks"]["queue"]["ok"], true);

    // Liveness does not depend on dependencies.
    let health = app
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(json_body(health).await["ok"], true);
}

#[tokio::test(start_paused = true)]
async fn a_hanging_check_fails_the_readiness_instead_of_hanging_it() {
    let hang = || async {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        Ok(())
    };
    let checks: Vec<(String, Arc<dyn ReadyCheck>)> = vec![("upstream".into(), Arc::new(hang))];
    let app = health_router(checks);
    let response = app
        .oneshot(Request::get("/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        json_body(response).await["checks"]["upstream"]["error"]["code"],
        "READY_TIMEOUT"
    );
}
