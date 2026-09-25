use carmy_core::*;
use carmy_runtime::{Runtime, execution_request};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl<'a> MakeWriter<'a> for Buffer {
    type Writer = Buffer;
    fn make_writer(&'a self) -> Buffer {
        self.clone()
    }
}
struct Login;
impl Tool for Login {
    type Input = String;
    type Output = String;
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: "login".into(),
            description: "login".into(),
            input_schema: schemars::schema_for!(String).to_value(),
            output_schema: schemars::schema_for!(String).to_value(),
            effect: Effect::ExternalWrite,
            idempotent: false,
            parallel_safe: true,
            confirmation: Confirmation::None,
        }
    }
    async fn execute(&self, _: AgentContext, _: String) -> AgentResult<String> {
        Ok("token-abc".into())
    }
}
#[tokio::test(flavor = "current_thread")]
async fn executions_are_traced_without_arguments_or_outputs() {
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buffer.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let runtime = Runtime::new().tool(Login).unwrap();
    let mut request = execution_request("login", json!("hunter2"));
    request.request_id = Some("req-1".into());
    let result = runtime.execute(request.clone()).await;
    assert!(result.outcome.is_ok());
    runtime.execute(request).await;
    let log = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    let span = carmy_observability::EXECUTION_SPAN;
    for expected in [
        span,
        &format!("execution_id={}", result.execution_id),
        "request_id=\"req-1\"",
        "tool=login",
        "effect=\"external_write\"",
        "status=\"completed\"",
        "duration_ms=",
        "replayed=true",
    ] {
        assert!(log.contains(expected), "missing {expected} in {log}");
    }
    assert!(!log.contains("hunter2"), "arguments must not be logged");
    assert!(!log.contains("token-abc"), "outputs must not be logged");
}
