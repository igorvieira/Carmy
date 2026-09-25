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
