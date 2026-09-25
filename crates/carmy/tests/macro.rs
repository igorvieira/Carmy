use carmy::prelude::*;
#[derive(Deserialize, JsonSchema)]
struct Input {
    query: String,
}
#[derive(Serialize, JsonSchema)]
struct Output {
    result: String,
}
#[carmy::tool(
    description = "Search",
    effect = "read",
    idempotent = true,
    parallel_safe = true
)]
async fn search(_: AgentContext, input: Input) -> AgentResult<Output> {
    Ok(Output {
        result: input.query,
    })
}
#[tokio::test]
async fn generated_tool_executes_and_describes_itself() {
    let metadata = search.metadata();
    assert_eq!(metadata.name, "search");
    assert_eq!(metadata.effect, Effect::Read);
    assert!(metadata.idempotent && metadata.parallel_safe);
    assert_eq!(
        metadata.input_schema["properties"]["query"]["type"],
        "string"
    );
    assert_eq!(
        search
            .execute(
                AgentContext::default(),
                Input {
                    query: "hello".into()
                }
            )
            .await
            .unwrap()
            .result,
        "hello"
    );
}
#[test]
fn diagnostics() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/valid.rs");
    t.compile_fail("tests/ui/invalid_*.rs");
}
#[derive(Deserialize, JsonSchema)]
#[allow(dead_code)]
struct DeleteInput {
    id: String,
}
#[carmy::tool(
    description = "Delete a customer",
    effect = "destructive",
    confirmation = "required"
)]
async fn delete_customer(_: AgentContext, _: DeleteInput) -> AgentResult<()> {
    Ok(())
}
#[test]
fn destructive_tools_are_identifiable_before_execution() {
    let m = delete_customer.metadata();
    assert_eq!(m.effect, Effect::Destructive);
    assert_eq!(m.confirmation, Confirmation::Required);
    assert!(!m.idempotent && !m.parallel_safe);
}
#[tokio::test]
async fn facade_builds_runtime_and_reports_registration_errors() {
    let runtime = Carmy::new().tool(search).build().unwrap();
    let request = carmy::runtime::execution_request("search", serde_json::json!({"query": "q"}));
    assert_eq!(
        runtime.execute(request).await.outcome.unwrap()["result"],
        "q"
    );
    let duplicate = Carmy::new().tool(search).tool(search).build();
    assert!(matches!(
        duplicate,
        Err(carmy::Error::Registration(e)) if e.code == "DUPLICATE_TOOL"
    ));
}
