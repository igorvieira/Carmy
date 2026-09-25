use carmy_core::*;
use carmy_runtime::*;
use serde_json::json;
struct Echo(Effect);
impl Tool for Echo {
    type Input = String;
    type Output = String;
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: "echo".into(),
            description: "echo".into(),
            input_schema: schemars::schema_for!(String).to_value(),
            output_schema: schemars::schema_for!(String).to_value(),
            effect: self.0,
            idempotent: true,
            parallel_safe: true,
            confirmation: Confirmation::None,
        }
    }
    async fn execute(&self, _: AgentContext, input: String) -> AgentResult<String> {
        Ok(input)
    }
}
fn request(arguments: serde_json::Value) -> ExecutionRequest {
    ExecutionRequest {
        execution_id: "exec-1".into(),
        request_id: None,
        tool: "echo".into(),
        arguments,
        context: AgentContext::default(),
        metadata: Default::default(),
    }
}
#[tokio::test]
async fn execution_validation_resolution_and_policy() {
    let runtime = Runtime::new().tool(Echo(Effect::Read)).unwrap();
    assert_eq!(
        runtime.execute(request(json!("hi"))).await.outcome.unwrap(),
        "hi"
    );
    assert_eq!(
        runtime
            .execute(request(json!(4)))
            .await
            .outcome
            .unwrap_err()
            .code,
        "INVALID_ARGUMENTS"
    );
    let mut missing = request(json!("hi"));
    missing.tool = "missing".into();
    assert_eq!(
        runtime.execute(missing).await.outcome.unwrap_err().code,
        "TOOL_NOT_FOUND"
    );
    let runtime = Runtime::new().tool(Echo(Effect::Destructive)).unwrap();
    assert_eq!(
        runtime
            .execute(request(json!("hi")))
            .await
            .outcome
            .unwrap_err()
            .code,
        "CONFIRMATION_REQUIRED"
    );
    let mut confirmed = request(json!("hi"));
    confirmed.context.permissions.insert("confirm:echo".into());
    assert!(runtime.execute(confirmed).await.outcome.is_ok());
}
#[test]
fn duplicate_names_are_rejected() {
    assert!(
        Runtime::new()
            .tool(Echo(Effect::Read))
            .unwrap()
            .tool(Echo(Effect::Read))
            .is_err()
    );
}
