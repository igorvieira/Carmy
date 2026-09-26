use axum::{body::Body, http::Request};
use carmy::http::Webhook;
use carmy::prelude::*;
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

#[derive(Deserialize, JsonSchema)]
struct Event {
    id: String,
}
#[carmy::tool(
    description = "Handle a billing event",
    effect = "write",
    register = false
)]
async fn billing_event(input: Event) -> AgentResult<String> {
    Ok(input.id)
}

#[tokio::test]
async fn an_app_receives_webhooks_and_queues_them_as_jobs() {
    let store = std::sync::Arc::new(carmy::jobs::InMemoryJobStore::default());
    let app = Carmy::new()
        .tool(billing_event)
        .jobs(store.clone())
        .webhook(
            "/webhooks/billing",
            Webhook::hmac_sha256("s3cret", "X-Signature")
                .tool("billing_event")
                .event_id("/id")
                .enqueue(),
        )
        .router()
        .unwrap();
    let body = r#"{"id":"evt_1"}"#;
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"s3cret").unwrap();
    mac.update(body.as_bytes());
    let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
    let response = app
        .oneshot(
            Request::post("/webhooks/billing")
                .header("x-signature", signature)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    let accepted: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(accepted["request_id"], "evt_1");
    let id = carmy::jobs::JobId(accepted["job_id"].as_str().unwrap().to_owned());
    let job = carmy::jobs::JobStore::get(&*store, &id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.request.tool, "billing_event");
    assert_eq!(job.request.request_id.as_deref(), Some("evt_1"));
}
