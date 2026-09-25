use carmy_core::*;
use carmy_mcp::{McpServer, REQUEST_ID_META};
use carmy_runtime::Runtime;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, MetaObject, RequestMetaObject},
    service::{RoleClient, RunningService},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Deserialize, JsonSchema)]
struct Input {
    name: String,
}
#[derive(Serialize, JsonSchema)]
struct Output {
    id: usize,
}
struct Create(Arc<AtomicUsize>);
impl Tool for Create {
    type Input = Input;
    type Output = Output;
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: "create_user".into(),
            description: "Create a user".into(),
            input_schema: schemars::schema_for!(Input).to_value(),
            output_schema: schemars::schema_for!(Output).to_value(),
            effect: Effect::Write,
            idempotent: false,
            parallel_safe: true,
            confirmation: Confirmation::None,
        }
    }
    async fn execute(&self, _: AgentContext, input: Input) -> AgentResult<Output> {
        if input.name.is_empty() {
            let mut e =
                AgentError::new("EMPTY_NAME", "Name is required", ErrorCategory::Validation);
            e.recoverable = true;
            return Err(e);
        }
        Ok(Output {
            id: self.0.fetch_add(1, Ordering::SeqCst),
        })
    }
}
async fn connect(calls: Arc<AtomicUsize>) -> RunningService<RoleClient, ()> {
    let runtime = Arc::new(Runtime::new().tool(Create(calls)).unwrap());
    let (server, client) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let running = McpServer::new(runtime).serve(server).await.unwrap();
        running.waiting().await.unwrap();
    });
    ().serve(client).await.unwrap()
}
fn call(name: &str, arguments: serde_json::Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name.to_owned())
        .with_arguments(arguments.as_object().unwrap().clone())
}
#[tokio::test]
async fn exposes_tools_with_effect_annotations() {
    let client = connect(Arc::default()).await;
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 1);
    let tool = &tools[0];
    assert_eq!(tool.name, "create_user");
    assert_eq!(tool.input_schema["properties"]["name"]["type"], "string");
    assert!(tool.output_schema.is_some());
    let annotations = tool.annotations.as_ref().unwrap();
    assert_eq!(annotations.read_only_hint, Some(false));
    assert_eq!(annotations.destructive_hint, Some(false));
    assert_eq!(annotations.idempotent_hint, Some(false));
    assert_eq!(tool.meta.as_ref().unwrap().0["carmy/effect"], "write");
}
#[tokio::test]
async fn invokes_runtime_and_converts_errors() {
    let calls = Arc::new(AtomicUsize::new(0));
    let client = connect(calls.clone()).await;
    let ok = client
        .call_tool(call("create_user", json!({"name":"ada"})))
        .await
        .unwrap();
    assert_eq!(ok.is_error, Some(false));
    assert_eq!(ok.structured_content.unwrap()["id"], 0);
    assert_eq!(ok.meta.unwrap().0["carmy/status"], "completed");
    let failed = client
        .call_tool(call("create_user", json!({"name":""})))
        .await
        .unwrap();
    assert_eq!(failed.is_error, Some(true));
    let error = &failed.structured_content.unwrap()["error"];
    assert_eq!(error["code"], "EMPTY_NAME");
    assert_eq!(error["recoverable"], true);
    let invalid = client
        .call_tool(call("create_user", json!({"name": 4})))
        .await
        .unwrap();
    assert_eq!(
        invalid.structured_content.unwrap()["error"]["code"],
        "INVALID_ARGUMENTS"
    );
    assert!(client.call_tool(call("missing", json!({}))).await.is_err());
}
#[tokio::test]
async fn request_id_meta_makes_retries_safe() {
    let calls = Arc::new(AtomicUsize::new(0));
    let client = connect(calls.clone()).await;
    let mut params = call("create_user", json!({"name":"ada"}));
    let mut meta = MetaObject::new();
    meta.0.insert(REQUEST_ID_META.into(), json!("abc"));
    params.meta = Some(RequestMetaObject(meta));
    let a = client.call_tool(params.clone()).await.unwrap();
    let b = client.call_tool(params).await.unwrap();
    assert_eq!(a.structured_content, b.structured_content);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
struct Wait(Arc<tokio::sync::Notify>);
impl Tool for Wait {
    type Input = Input;
    type Output = Output;
    fn metadata(&self) -> ToolMetadata {
        let mut m = Create(Arc::default()).metadata();
        m.name = "wait".into();
        m
    }
    async fn execute(&self, ctx: AgentContext, _: Input) -> AgentResult<Output> {
        let notify = self.0.clone();
        tokio::spawn(async move {
            ctx.cancellation.cancelled().await;
            notify.notify_one();
        });
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        Ok(Output { id: 0 })
    }
}
#[tokio::test]
async fn mcp_cancellation_reaches_the_execution() {
    use rmcp::{
        model::{CallToolRequest, ClientRequest},
        service::PeerRequestOptions,
    };
    let cancelled = Arc::new(tokio::sync::Notify::new());
    let runtime = Arc::new(Runtime::new().tool(Wait(cancelled.clone())).unwrap());
    let (server, client) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let running = McpServer::new(runtime).serve(server).await.unwrap();
        running.waiting().await.unwrap();
    });
    let client = ().serve(client).await.unwrap();
    let request =
        ClientRequest::CallToolRequest(CallToolRequest::new(call("wait", json!({"name":"x"}))));
    let handle = client
        .peer()
        .send_cancellable_request(request, PeerRequestOptions::no_options())
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    handle.cancel(None).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), cancelled.notified())
        .await
        .expect("MCP cancellation must cancel the execution token");
}
