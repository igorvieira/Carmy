//! The tools every app offers about itself, over HTTP like any other tool.
use axum::{body::Body, http::Request};
use carmy::http::{Webhook, verify};
use carmy::prelude::*;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

#[derive(Deserialize, JsonSchema)]
struct Event {
    id: String,
    secret: String,
}
#[carmy::tool(description = "Handle an event", effect = "write", register = false)]
async fn handle_event(input: Event) -> AgentResult<String> {
    if input.id == "fails" {
        return Err(AgentError::new(
            "NOPE",
            "permanent",
            ErrorCategory::Validation,
        ));
    }
    Ok(input.secret.len().to_string())
}

async fn call(app: &axum::Router, tool: &str, arguments: Value) -> (u16, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::post("/agent/execute")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"tool": tool, "arguments": arguments}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn an_agent_follows_an_enqueued_delivery_with_carmy_job() {
    let (app, jobs) = Carmy::new()
        .tool(handle_event)
        .webhook(
            "/hook",
            Webhook::to("handle_event")
                .verify(verify::shared_secret("X-Token", "t"))
                .event_id("/id")
                .enqueue(),
        )
        .router_and_jobs()
        .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::post("/hook")
                .header("x-token", "t")
                .body(Body::from(r#"{"id":"e1","secret":"hunter2"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    let accepted: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let job_id = accepted["job_id"].clone();

    let (status, queued) = call(&app, "carmy_job", json!({ "job_id": job_id })).await;
    assert_eq!(status, 200);
    assert_eq!(queued["data"]["status"], "queued");
    assert_eq!(queued["data"]["tool"], "handle_event");
    assert_eq!(queued["data"]["request_id"], "e1");
    assert!(
        !queued.to_string().contains("hunter2"),
        "never the arguments"
    );
    assert_eq!(
        queued["_agent"]["cacheable"], false,
        "status changes over time"
    );

    jobs.run_due(10).await.unwrap();
    let (_, done) = call(&app, "carmy_job", json!({ "job_id": job_id })).await;
    assert_eq!(done["data"]["status"], "succeeded");
    assert_eq!(done["data"]["attempts"], 1);

    let (status, missing) = call(&app, "carmy_job", json!({ "job_id": "job_nope" })).await;
    assert_eq!(status, 404);
    assert_eq!(missing["error"]["code"], "JOB_NOT_FOUND");
}

#[tokio::test]
async fn operator_tools_are_opt_in_and_obey_policies() {
    let plain = Carmy::new().tool(handle_event).build().unwrap();
    assert!(plain.metadata("carmy_job").is_some());
    assert!(plain.metadata("carmy_dead_letters").is_none());
    assert!(plain.metadata("carmy_audit").is_none());

    let (app, jobs) = Carmy::new()
        .tool(handle_event)
        .operator_tools()
        .router_and_jobs()
        .unwrap();
    let failing = carmy::runtime::execution_request(
        "handle_event",
        json!({"id": "fails", "secret": "hunter2"}),
    );
    let id = jobs.enqueue(failing).await.unwrap();
    jobs.run_due(10).await.unwrap();
    assert_eq!(
        jobs.get(&id).await.unwrap().unwrap().status.as_str(),
        "failed"
    );

    let (_, audit) = call(&app, "carmy_audit", json!({ "limit": 5 })).await;
    let executions = audit["data"]["executions"].as_array().unwrap();
    assert_eq!(executions[0]["tool"], "handle_event");
    assert_eq!(executions[0]["error_code"], "NOPE");
    assert!(!audit.to_string().contains("hunter2"), "never the payload");
    let (_, dead) = call(&app, "carmy_dead_letters", json!({})).await;
    assert_eq!(
        dead["data"]["jobs"],
        json!([]),
        "failed is not dead-lettered"
    );

    // They are tools like any other: a permission policy guards them.
    let guarded = Carmy::new()
        .tool(handle_event)
        .operator_tools()
        .policy(carmy::runtime::RequireToolPermission)
        .router()
        .unwrap();
    let (status, denied) = call(&guarded, "carmy_audit", json!({})).await;
    assert_eq!(status, 403);
    assert_eq!(denied["error"]["code"], "FORBIDDEN");
}

#[carmy::tool(
    description = "Shadows a system tool",
    effect = "read",
    register = false
)]
async fn carmy_job() -> AgentResult<()> {
    Ok(())
}

#[test]
fn system_tool_names_are_reserved() {
    let error = Carmy::new()
        .tool(carmy_job)
        .build()
        .err()
        .expect("a configuration error");
    assert!(error.to_string().contains("registration"), "{error}");
}
