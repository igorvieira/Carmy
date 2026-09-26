use carmy::prelude::*;

#[derive(Deserialize, JsonSchema)]
struct {{Pascal}}Input {
    /// Describe each field: agents read these comments in the schema.
    value: String,
}

#[derive(Serialize, JsonSchema)]
struct {{Pascal}}Output {
    value: String,
}

{{attribute}}
async fn {{name}}(input: {{Pascal}}Input) -> AgentResult<{{Pascal}}Output> {
    Ok({{Pascal}}Output { value: input.value })
}

#[cfg(test)]
mod tests {
    use super::*;
    use carmy::serde_json::json;

    #[tokio::test]
    async fn {{test}}() {
        let result = carmy::testing::execute({{name}}, json!({ "value": "hello" })).await;
{{assertion}}
    }
}
