use carmy_core::*;
use carmy_runtime::{InMemoryAudit, RequireToolPermission, Runtime, execution_request};
use serde_json::json;
use std::sync::Arc;

struct Echo;
impl Tool for Echo {
    type Input = String;
    type Output = String;
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: "echo".into(),
            description: "echo".into(),
            input_schema: schemars::schema_for!(String).to_value(),
            output_schema: schemars::schema_for!(String).to_value(),
            effect: Effect::ExternalWrite,
            idempotent: true,
            parallel_safe: true,
            confirmation: Confirmation::None,
        }
    }
    async fn execute(&self, _: AgentContext, input: String) -> AgentResult<String> {
        if input == "boom" {
            return Err(AgentError::new("BOOM", "no", ErrorCategory::Validation));
        }
        Ok(input)
    }
}

fn request(input: &str, principal: Option<&str>) -> ExecutionRequest {
    let mut request = execution_request("echo", json!(input));
    request.context.principal = principal.map(str::to_owned);
    request.context.session = Some("s1".into());
    request
}

#[tokio::test]
async fn every_execution_leaves_one_record_without_payloads() {
    let audit = Arc::new(InMemoryAudit::new(10));
    let runtime = Runtime::new().sink(audit.clone()).tool(Echo).unwrap();

    let ok = runtime.execute(request("secret-input", Some("ada"))).await;
    let failed = runtime.execute(request("boom", Some("ada"))).await;
    let replayed = request("again", None).with_request_id("r1");
    runtime.execute(replayed.clone()).await;
    runtime.execute(replayed).await;
    runtime
        .execute(execution_request("missing", json!({})))
        .await;

    let records = audit.recent(10);
    assert_eq!(
        records.len(),
        5,
        "one record per execution, replays included"
    );
    // Newest first.
    let missing = &records[0];
    assert_eq!(missing.tool, "missing");
    assert_eq!(missing.effect, None);
    assert_eq!(missing.status, ExecutionStatus::Failed);
    assert_eq!(missing.error_code.as_deref(), Some("TOOL_NOT_FOUND"));
    let replay = &records[1];
    assert!(replay.replayed);
    assert_eq!(replay.request_id.as_deref(), Some("r1"));
    assert_eq!(
        replay.execution_id, records[2].execution_id,
        "a replay reports the original"
    );
    assert!(!records[2].replayed);
    let boom = &records[3];
    assert_eq!(boom.error_code.as_deref(), Some("BOOM"));
    assert_eq!(boom.execution_id, failed.execution_id);
    let first = &records[4];
    assert_eq!(first.execution_id, ok.execution_id);
    assert_eq!(first.principal.as_deref(), Some("ada"));
    assert_eq!(first.session.as_deref(), Some("s1"));
    assert_eq!(first.effect, Some(Effect::ExternalWrite));
    assert_eq!(first.status, ExecutionStatus::Completed);
    assert_eq!(first.error_code, None);

    let text = serde_json::to_string(&records).unwrap();
    assert!(
        !text.contains("secret-input"),
        "arguments never reach the audit trail"
    );
    assert!(!text.contains("arguments") && !text.contains("data"));
}

#[tokio::test]
async fn policy_rejections_are_recorded_and_the_buffer_is_bounded() {
    let audit = Arc::new(InMemoryAudit::new(2));
    let runtime = Runtime::new()
        .sink(audit.clone())
        .policy(RequireToolPermission)
        .tool(Echo)
        .unwrap();
    for _ in 0..3 {
        runtime.execute(request("x", Some("ada"))).await;
    }
    let records = audit.recent(10);
    assert_eq!(records.len(), 2, "capacity keeps the newest");
    assert!(
        records
            .iter()
            .all(|r| r.error_code.as_deref() == Some("FORBIDDEN"))
    );
    assert!(records.iter().all(|r| r.status == ExecutionStatus::Failed));
}
