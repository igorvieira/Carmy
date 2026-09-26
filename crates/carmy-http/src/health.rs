//! Liveness and readiness. `GET /health` says the process answers; `GET /ready` runs
//! every registered check (a database, a queue's worker, an upstream) and answers
//! 503 with the failing names, so a load balancer or orchestrator routes around a
//! server whose dependencies are gone.
use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use carmy_core::{AgentError, AgentResult, ErrorCategory};
use serde_json::{Map, json};
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

pub const HEALTH_PATH: &str = "/health";
pub const READY_PATH: &str = "/ready";
/// Longest a single check may take before it counts as failed.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// One dependency the server needs before it can do useful work.
pub trait ReadyCheck: Send + Sync {
    fn check(&self) -> Pin<Box<dyn Future<Output = AgentResult<()>> + Send + '_>>;
}

impl<F, Fut> ReadyCheck for F
where
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = AgentResult<()>> + Send + 'static,
{
    fn check(&self) -> Pin<Box<dyn Future<Output = AgentResult<()>> + Send + '_>> {
        Box::pin(self())
    }
}

#[derive(Clone, Default)]
struct Checks(Arc<Vec<(String, Arc<dyn ReadyCheck>)>>);

/// A router with [`HEALTH_PATH`] and [`READY_PATH`]. Merge it with the agent router.
pub fn health_router(checks: impl IntoIterator<Item = (String, Arc<dyn ReadyCheck>)>) -> Router {
    Router::new()
        .route(HEALTH_PATH, get(health))
        .route(READY_PATH, get(ready))
        .with_state(Checks(Arc::new(checks.into_iter().collect())))
}

async fn health() -> Response {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "ok": true })),
    )
        .into_response()
}

async fn ready(State(checks): State<Checks>) -> Response {
    let mut results = Map::new();
    let mut all_ok = true;
    // Checks run together: readiness must answer quickly even with a slow dependency.
    let outcomes =
        futures_util::future::join_all(checks.0.iter().map(|(name, check)| async move {
            let outcome = match tokio::time::timeout(CHECK_TIMEOUT, check.check()).await {
                Ok(outcome) => outcome,
                Err(_) => Err(AgentError::new(
                    "READY_TIMEOUT",
                    format!("`{name}` did not answer within {CHECK_TIMEOUT:?}"),
                    ErrorCategory::Timeout,
                )),
            };
            (name.clone(), outcome)
        }))
        .await;
    for (name, outcome) in outcomes {
        let entry = match outcome {
            Ok(()) => json!({ "ok": true }),
            Err(error) => {
                all_ok = false;
                json!({ "ok": false, "error": error })
            }
        };
        results.insert(name, entry);
    }
    let status = if all_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "ready": all_ok, "checks": results })),
    )
        .into_response()
}
