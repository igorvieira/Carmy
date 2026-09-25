use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use carmy_core::*;
use carmy_runtime::Runtime;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tower::ServiceExt;
struct Counter(Arc<AtomicUsize>);
impl Tool for Counter {
    type Input = String;
    type Output = usize;
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: "count".into(),
            description: "count".into(),
            input_schema: schemars::schema_for!(String).to_value(),
            output_schema: schemars::schema_for!(usize).to_value(),
            effect: Effect::Write,
            idempotent: false,
            parallel_safe: false,
            confirmation: Confirmation::None,
        }
    }
    async fn execute(&self, _: AgentContext, _: String) -> AgentResult<usize> {
        Ok(self.0.fetch_add(1, Ordering::SeqCst))
    }
}
fn app() -> axum::Router {
    carmy_http::router(
        Arc::new(
            Runtime::new()
                .tool(Counter(Arc::new(AtomicUsize::new(0))))
                .unwrap(),
        ),
        "test",
    )
}
fn post(value: Value) -> Request<Body> {
    Request::post("/agent/execute")
        .header("content-type", "application/json")
        .body(Body::from(value.to_string()))
        .unwrap()
}
async fn json_body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
#[tokio::test]
async fn discover_list_execute_retry_and_conflict() {
    let app = app();
    let response = app
        .clone()
        .oneshot(
            Request::get("/.well-known/agent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(json_body(response).await["protocol"], "carmy/1");
    let response = app
        .clone()
        .oneshot(Request::get("/agent/tools").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let etag = response.headers()["etag"].clone();
    assert_eq!(json_body(response).await["tools"][0]["name"], "count");
    let cached = app
        .clone()
        .oneshot(
            Request::get("/agent/tools")
                .header("if-none-match", etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);
    let request = json!({"tool":"count","arguments":"x","request_id":"abc"});
    let a = json_body(app.clone().oneshot(post(request.clone())).await.unwrap()).await;
    let b = json_body(app.clone().oneshot(post(request)).await.unwrap()).await;
    assert_eq!(a, b);
    assert_eq!(a["data"], 0);
    let conflict = app
        .clone()
        .oneshot(post(
            json!({"tool":"count","arguments":"changed","request_id":"abc"}),
        ))
        .await
        .unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(conflict).await["error"]["code"],
        "IDEMPOTENCY_CONFLICT"
    );
}
#[tokio::test]
async fn failures_and_untrusted_context_are_structured() {
    for value in [
        json!({"tool":"count","arguments":4}),
        json!({"tool":"count","context":{"permissions":["admin"]}}),
    ] {
        let response = app().oneshot(post(value)).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(json_body(response).await["error"]["code"].is_string());
    }
    let response = app().oneshot(post(json!({"tool":"absent"}))).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
struct Slow;
impl Tool for Slow {
    type Input = String;
    type Output = String;
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: "slow".into(),
            description: "slow read".into(),
            input_schema: schemars::schema_for!(String).to_value(),
            output_schema: schemars::schema_for!(String).to_value(),
            effect: Effect::Read,
            idempotent: true,
            parallel_safe: true,
            confirmation: Confirmation::None,
        }
    }
    async fn execute(&self, _: AgentContext, input: String) -> AgentResult<String> {
        if input == "wait" {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        }
        Ok(input)
    }
}
#[tokio::test]
async fn timeouts_limits_and_cacheability() {
    let runtime = Runtime::new()
        .timeout(std::time::Duration::from_millis(10))
        .tool(Slow)
        .unwrap();
    let app = carmy_http::router(Arc::new(runtime), "test");
    let ok = app
        .clone()
        .oneshot(post(json!({"tool":"slow","arguments":"now"})))
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    assert_eq!(ok.headers()["cache-control"], "no-store");
    let ok = json_body(ok).await;
    assert_eq!(ok["status"], "completed");
    assert_eq!(ok["_agent"]["cacheable"], true);
    let timed = app
        .clone()
        .oneshot(post(json!({"tool":"slow","arguments":"wait"})))
        .await
        .unwrap();
    assert_eq!(timed.status(), StatusCode::GATEWAY_TIMEOUT);
    let timed = json_body(timed).await;
    assert_eq!(timed["status"], "timed_out");
    assert_eq!(timed["error"]["code"], "TIMEOUT");
    assert_eq!(timed["_agent"]["cacheable"], false);
    let huge = "x".repeat(2 * 1024 * 1024);
    let response = app
        .oneshot(post(json!({"tool":"slow","arguments":huge})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "PAYLOAD_TOO_LARGE"
    );
}
fn stream(value: Value) -> Request<Body> {
    Request::post("/agent/execute")
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .body(Body::from(value.to_string()))
        .unwrap()
}
#[tokio::test]
async fn sse_streams_runtime_events() {
    let response = app()
        .oneshot(stream(
            json!({"tool":"count","arguments":"x","request_id":"s"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body = std::str::from_utf8(&body).unwrap();
    let events: Vec<_> = body
        .lines()
        .filter_map(|l| l.strip_prefix("event: "))
        .collect();
    assert_eq!(
        events,
        [
            "execution.started",
            "tool.started",
            "tool.completed",
            "execution.completed"
        ]
    );
    let last: Value = serde_json::from_str(
        body.lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .next_back()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(last["status"], "completed");
    assert_eq!(last["data"], 0);
    assert_eq!(last["replayed"], false);
}
struct Watch(Arc<tokio::sync::Notify>);
impl Tool for Watch {
    type Input = String;
    type Output = String;
    fn metadata(&self) -> ToolMetadata {
        let mut m = Slow.metadata();
        m.name = "watch".into();
        m
    }
    async fn execute(&self, ctx: AgentContext, input: String) -> AgentResult<String> {
        // Background work observes the execution's cancellation token.
        let notify = self.0.clone();
        tokio::spawn(async move {
            ctx.cancellation.cancelled().await;
            notify.notify_one();
        });
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        Ok(input)
    }
}
#[tokio::test]
async fn client_disconnect_cancels_streamed_execution() {
    let cancelled = Arc::new(tokio::sync::Notify::new());
    let app = carmy_http::router(
        Arc::new(Runtime::new().tool(Watch(cancelled.clone())).unwrap()),
        "test",
    );
    let response = app
        .oneshot(stream(json!({"tool":"watch","arguments":"x"})))
        .await
        .unwrap();
    let mut body = response.into_body();
    // Read until the tool has started, then disconnect.
    let mut seen = String::new();
    while !seen.contains("tool.started") {
        let frame = body.frame().await.unwrap().unwrap();
        seen.push_str(std::str::from_utf8(frame.data_ref().unwrap()).unwrap());
    }
    drop(body);
    tokio::time::timeout(std::time::Duration::from_secs(1), cancelled.notified())
        .await
        .expect("disconnect must cancel the execution token");
}
