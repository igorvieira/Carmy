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
#[derive(Clone)]
struct Greeting(&'static str);
#[derive(Serialize, JsonSchema)]
struct Greeted {
    message: String,
}
/// Greets with the configured greeting.
#[carmy::tool(description = "Greet", effect = "none")]
async fn greet(State(greeting): State<Greeting>, input: Input) -> AgentResult<Greeted> {
    Ok(Greeted {
        message: format!("{} {}", greeting.0, input.query),
    })
}
#[carmy::tool(effect = "read")]
async fn ping(ctx: AgentContext) -> AgentResult<String> {
    Ok(ctx.execution_id)
}
#[tokio::test]
async fn state_is_injected_and_checked_at_startup() {
    let app = Carmy::new().state(Greeting("Hello"));
    let result =
        carmy::testing::execute_with(app, greet, serde_json::json!({"query": "Ada"})).await;
    assert_eq!(result.outcome.unwrap()["message"], "Hello Ada");
    let missing = Carmy::new().tool(greet).build();
    let Err(carmy::Error::Registration(e)) = missing else {
        panic!("missing state must fail at startup")
    };
    assert_eq!(e.code, "MISSING_STATE");
    assert!(e.message.contains("Greeting"), "{}", e.message);
}
#[tokio::test]
async fn tools_without_input_accept_empty_arguments() {
    let runtime = Carmy::new().tool(ping).build().unwrap();
    assert_eq!(ping.metadata().input_schema["type"], "object");
    let ok = runtime
        .execute(carmy::runtime::execution_request(
            "ping",
            serde_json::json!({}),
        ))
        .await;
    assert!(ok.outcome.unwrap().as_str().unwrap().starts_with("exec_"));
    let extra = runtime
        .execute(carmy::runtime::execution_request(
            "ping",
            serde_json::json!({"x": 1}),
        ))
        .await;
    assert_eq!(extra.outcome.unwrap_err().code, "INVALID_ARGUMENTS");
}
#[carmy::tool(effect = "none", register = false)]
async fn hidden() -> AgentResult<()> {
    Ok(())
}
#[test]
fn app_collects_declared_tools() {
    let runtime = carmy::app().state(Greeting("Hi")).build().unwrap();
    let names: Vec<_> = runtime.tools().into_iter().map(|t| t.name).collect();
    assert_eq!(names, ["delete_customer", "greet", "ping", "search"]);
    let _ = hidden;
    // Declared tools still need their state when collected automatically.
    assert!(matches!(
        carmy::app().build(),
        Err(carmy::Error::Registration(e)) if e.code == "MISSING_STATE"
    ));
}
