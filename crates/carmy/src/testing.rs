//! Helpers for testing tools through the full runtime pipeline
//! (schema validation, policies, output checks), without a transport.
use crate::{Carmy, ExecutionResult, IntoTool, Tool};
use serde_json::Value;

/// Execute `tool` once with `arguments`.
///
/// ```ignore
/// let result = carmy::testing::execute(hello, json!({ "name": "Ada" })).await;
/// assert_eq!(result.outcome.unwrap()["message"], "Hello, Ada!");
/// ```
pub async fn execute<T: IntoTool + 'static>(tool: T, arguments: Value) -> ExecutionResult {
    execute_with(Carmy::new(), tool, arguments).await
}

/// Execute `tool` in `app`, which supplies state (`.state(..)`) and policies.
///
/// # Panics
/// If the tool cannot be registered, e.g. because its state is missing.
pub async fn execute_with<T: IntoTool + 'static>(
    app: Carmy,
    tool: T,
    arguments: Value,
) -> ExecutionResult {
    let tool = tool
        .into_tool(&app.states)
        .unwrap_or_else(|e| panic!("cannot register tool: {e}"));
    let name = tool.metadata().name;
    let runtime = app
        .tool(tool)
        .build()
        .unwrap_or_else(|e| panic!("cannot build runtime: {e}"));
    runtime
        .execute(carmy_runtime::execution_request(name, arguments))
        .await
}
