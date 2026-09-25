use carmy::prelude::*;

#[derive(Deserialize, JsonSchema)]
struct HelloInput {
    /// Who to greet.
    name: String,
}

#[derive(Serialize, JsonSchema)]
struct HelloOutput {
    message: String,
}

#[carmy::tool(
    description = "Greet someone by name",
    effect = "none",
    idempotent = true,
    parallel_safe = true
)]
async fn hello(input: HelloInput) -> AgentResult<HelloOutput> {
    if input.name.trim().is_empty() {
        return Err(
            AgentError::new("EMPTY_NAME", "Name is required", ErrorCategory::Validation)
                .recoverable(),
        );
    }
    Ok(HelloOutput {
        message: format!("Hello, {}!", input.name),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use carmy::serde_json::json;

    #[tokio::test]
    async fn greets_by_name() {
        let result = carmy::testing::execute(hello, json!({ "name": "Ada" })).await;
        assert_eq!(result.outcome.unwrap()["message"], "Hello, Ada!");
    }

    #[tokio::test]
    async fn rejects_empty_names() {
        let result = carmy::testing::execute(hello, json!({ "name": " " })).await;
        assert_eq!(result.outcome.unwrap_err().code, "EMPTY_NAME");
    }
}
