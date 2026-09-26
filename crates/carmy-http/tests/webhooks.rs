use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use carmy_core::*;
use carmy_http::{Enqueue, Webhook, webhook_router};
use carmy_runtime::Runtime;
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tower::ServiceExt;

struct Record(Arc<AtomicUsize>);
impl Tool for Record {
    type Input = Value;
    type Output = usize;
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: "handle_event".into(),
            description: "records an event".into(),
            input_schema: json!({}),
            output_schema: schemars::schema_for!(usize).to_value(),
            effect: Effect::Write,
            idempotent: false,
            parallel_safe: true,
            confirmation: Confirmation::None,
        }
    }
    async fn execute(&self, _: AgentContext, _: Value) -> AgentResult<usize> {
        Ok(self.0.fetch_add(1, Ordering::SeqCst) + 1)
    }
}

/// A queue that remembers requests and dedupes on request_id, like `Jobs` does.
#[derive(Default)]
struct FakeQueue(Mutex<HashMap<String, ExecutionRequest>>);
impl Enqueue for FakeQueue {
    fn enqueue<'a>(
        &'a self,
        request: ExecutionRequest,
    ) -> Pin<Box<dyn Future<Output = AgentResult<String>> + Send + 'a>> {
        Box::pin(async move {
            let id = request
                .request_id
                .clone()
                .unwrap_or_else(|| "anonymous".into());
            self.0.lock().unwrap().entry(id.clone()).or_insert(request);
            Ok(format!("job_{id}"))
        })
    }
}

fn runtime(calls: &Arc<AtomicUsize>) -> Arc<Runtime> {
    Arc::new(Runtime::new().tool(Record(calls.clone())).unwrap())
}
fn sign(secret: &str, body: &str) -> String {
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}
fn deliver(path: &str, header: (&str, String), body: &str) -> Request<Body> {
    Request::post(path)
        .header("content-type", "application/json")
        .header(header.0, header.1)
        .body(Body::from(body.to_owned()))
        .unwrap()
}
async fn json_body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn a_redelivery_replays_instead_of_running_twice() {
    let calls = Arc::new(AtomicUsize::new(0));
    let hook = Webhook::hmac_sha256("s3cret", "X-Signature")
        .tool("handle_event")
        .event_id("/id");
    let app = webhook_router(
        runtime(&calls),
        [("/webhooks/billing".to_string(), hook)],
        None,
    )
    .unwrap();
    let body = r#"{"id":"evt_42","type":"subscription.created"}"#;
    let signed = ("x-signature", sign("s3cret", body));

    let first = app
        .clone()
        .oneshot(deliver("/webhooks/billing", signed.clone(), body))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let first = json_body(first).await;
    assert_eq!(first["data"], json!(1));

    let second = app
        .clone()
        .oneshot(deliver("/webhooks/billing", signed, body))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    let second = json_body(second).await;
    assert_eq!(
        second["execution_id"], first["execution_id"],
        "same execution, replayed"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the tool ran once");

    // Tampered, unsigned and non-JSON deliveries never reach the tool.
    let tampered = ("x-signature", sign("s3cret", body));
    let response = app
        .clone()
        .oneshot(deliver("/webhooks/billing", tampered, r#"{"id":"evt_43"}"#))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "WEBHOOK_UNAUTHORIZED"
    );
    let response = app
        .clone()
        .oneshot(deliver("/webhooks/billing", ("x-none", "1".into()), body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let not_json = "not json";
    let response = app
        .oneshot(deliver(
            "/webhooks/billing",
            ("x-signature", sign("s3cret", not_json)),
            not_json,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "INVALID_REQUEST"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn without_an_event_id_every_delivery_runs() {
    let calls = Arc::new(AtomicUsize::new(0));
    let hook = Webhook::shared_secret("X-Token", "t0ken").tool("handle_event");
    let app = webhook_router(runtime(&calls), [("/hook".to_string(), hook)], None).unwrap();
    for _ in 0..2 {
        let response = app
            .clone()
            .oneshot(deliver(
                "/hook",
                ("x-token", "t0ken".into()),
                r#"{"id":"same"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn enqueue_mode_accepts_at_once_and_dedupes_on_the_event_id() {
    let calls = Arc::new(AtomicUsize::new(0));
    let queue = Arc::new(FakeQueue::default());
    let hook = Webhook::shared_secret("X-Token", "t0ken")
        .tool("handle_event")
        .event_id("/update_id")
        .enqueue();
    let app = webhook_router(
        runtime(&calls),
        [("/webhooks/chat".to_string(), hook)],
        Some(queue.clone()),
    )
    .unwrap();
    let body = r#"{"update_id":7,"message":{"text":"/start"}}"#;
    let token = ("x-token", "t0ken".to_string());
    for _ in 0..2 {
        let response = app
            .clone()
            .oneshot(deliver("/webhooks/chat", token.clone(), body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let accepted = json_body(response).await;
        assert_eq!(accepted["job_id"], "job_7");
        assert_eq!(accepted["request_id"], "7");
    }
    {
        let queued = queue.0.lock().unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued["7"].tool, "handle_event");
        assert_eq!(queued["7"].arguments["message"]["text"], "/start");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing ran inline");

    let wrong = ("x-token", "guess".to_string());
    let response = app
        .oneshot(deliver("/webhooks/chat", wrong, body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn misconfigured_webhooks_fail_at_build_time() {
    let calls = Arc::new(AtomicUsize::new(0));
    let hook = || Webhook::shared_secret("X-Token", "t");
    let no_tool = webhook_router(runtime(&calls), [("/w".to_string(), hook())], None);
    assert_eq!(no_tool.err().unwrap().code, "INVALID_WEBHOOK");
    let unknown = webhook_router(
        runtime(&calls),
        [("/w".to_string(), hook().tool("nope"))],
        None,
    );
    assert!(unknown.err().unwrap().message.contains("unknown tool"));
    let no_queue = webhook_router(
        runtime(&calls),
        [("/w".to_string(), hook().tool("handle_event").enqueue())],
        None,
    );
    assert!(no_queue.err().unwrap().message.contains("no queue"));
}
