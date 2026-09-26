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
    // Successful results carry no `_meta` unless execution_meta is enabled.
    assert!(ok.meta.is_none());
    let failed = client
        .call_tool(call("create_user", json!({"name":""})))
        .await
        .unwrap();
    assert_eq!(failed.is_error, Some(true));
    // Errors must not set structuredContent: clients validate it against the output schema.
    assert!(failed.structured_content.is_none());
    let text: serde_json::Value =
        serde_json::from_str(&failed.content[0].as_text().unwrap().text).unwrap();
    assert_eq!(text["error"]["code"], "EMPTY_NAME");
    let error = &failed.meta.unwrap().0["carmy/error"];
    assert_eq!(error["code"], "EMPTY_NAME");
    assert_eq!(error["recoverable"], true);
    let invalid = client
        .call_tool(call("create_user", json!({"name": 4})))
        .await
        .unwrap();
    assert_eq!(
        invalid.meta.unwrap().0["carmy/error"]["code"],
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

#[tokio::test]
async fn execution_meta_is_opt_in_for_successes() {
    let runtime = Arc::new(Runtime::new().tool(Create(Arc::default())).unwrap());
    let (server, client) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let running = McpServer::new(runtime)
            .execution_meta(true)
            .serve(server)
            .await
            .unwrap();
        running.waiting().await.unwrap();
    });
    let client = ().serve(client).await.unwrap();
    let ok = client
        .call_tool(call("create_user", json!({"name":"ada"})))
        .await
        .unwrap();
    let meta = ok.meta.unwrap();
    assert_eq!(meta.0["carmy/status"], "completed");
    assert!(
        meta.0["carmy/execution_id"]
            .as_str()
            .unwrap()
            .starts_with("exec_")
    );
    // Errors carry execution metadata whether or not it is enabled.
    let failed = client
        .call_tool(call("create_user", json!({"name":""})))
        .await
        .unwrap();
    assert_eq!(failed.meta.unwrap().0["carmy/status"], "failed");
}
