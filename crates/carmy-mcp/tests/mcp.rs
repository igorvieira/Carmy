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

struct Import;
impl Tool for Import {
    type Input = Input;
    type Output = Output;
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: "import".into(),
            ..Create(Arc::new(AtomicUsize::new(0))).metadata()
        }
    }
    async fn execute(&self, ctx: AgentContext, _: Input) -> AgentResult<Output> {
        for page in 1..=3 {
            ctx.progress
                .report(page as f64, Some(3.0), format!("page {page}"));
        }
        Ok(Output { id: 3 })
    }
}

/// A client that records the progress notifications it receives.
#[derive(Clone, Default)]
struct Watcher(Arc<std::sync::Mutex<Vec<rmcp::model::ProgressNotificationParam>>>);
impl rmcp::ClientHandler for Watcher {
    async fn on_progress(
        &self,
        params: rmcp::model::ProgressNotificationParam,
        _: rmcp::service::NotificationContext<RoleClient>,
    ) {
        self.0.lock().unwrap().push(params);
    }
}

#[tokio::test]
async fn progress_tokens_receive_the_tools_reports() {
    let runtime = Arc::new(Runtime::new().tool(Import).unwrap());
    let (server, client) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let running = McpServer::new(runtime).serve(server).await.unwrap();
        running.waiting().await.unwrap();
    });
    let watcher = Watcher::default();
    let client = watcher.clone().serve(client).await.unwrap();

    // The rmcp client attaches a progress token to every call.
    let result = client
        .call_tool(call("import", json!({"name": "x"})))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(false));

    // Notifications are delivered asynchronously; give them a moment to land.
    for _ in 0..50 {
        if watcher.0.lock().unwrap().len() == 3 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let seen = watcher.0.lock().unwrap().clone();
    assert_eq!(seen.len(), 3);
    // The rmcp client assigns its own token; the server must echo whatever it received.
    assert!(
        seen.iter()
            .all(|p| p.progress_token == seen[0].progress_token)
    );
    assert_eq!(seen[2].progress, 3.0);
    assert_eq!(seen[2].total, Some(3.0));
    assert_eq!(seen[2].message.as_deref(), Some("page 3"));
}

/// A tool that takes a while, so calls to it can outlive the promotion delay.
struct Slow(std::time::Duration);
impl Tool for Slow {
    type Input = Input;
    type Output = Output;
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: "slow".into(),
            ..Create(Arc::new(AtomicUsize::new(0))).metadata()
        }
    }
    async fn execute(&self, ctx: AgentContext, _: Input) -> AgentResult<Output> {
        ctx.progress.report(0.0, None, "warming up");
        tokio::time::sleep(self.0).await;
        Ok(Output { id: 42 })
    }
}
struct Wipe;
impl Tool for Wipe {
    type Input = Input;
    type Output = Output;
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: "wipe".into(),
            effect: Effect::Destructive,
            confirmation: Confirmation::Required,
            ..Create(Arc::new(AtomicUsize::new(0))).metadata()
        }
    }
    async fn execute(&self, _: AgentContext, _: Input) -> AgentResult<Output> {
        Ok(Output { id: 0 })
    }
}

/// A client standing for a person: it may support tasks, and may answer elicitations.
#[derive(Clone)]
struct Person {
    tasks: bool,
    /// `None`: cannot be asked. `Some(answer)`: says yes or no.
    confirms: Option<bool>,
    asked: Arc<AtomicUsize>,
}
impl rmcp::ClientHandler for Person {
    fn get_info(&self) -> rmcp::model::ClientConfig {
        let capabilities = match (self.tasks, self.confirms.is_some()) {
            (true, true) => rmcp::model::ClientCapabilities::builder()
                .enable_elicitation()
                .enable_tasks()
                .build(),
            (true, false) => rmcp::model::ClientCapabilities::builder()
                .enable_tasks()
                .build(),
            (false, true) => rmcp::model::ClientCapabilities::builder()
                .enable_elicitation()
                .build(),
            (false, false) => rmcp::model::ClientCapabilities::default(),
        };
        rmcp::model::ClientConfig::new(
            capabilities,
            rmcp::model::Implementation::new("person", "1"),
        )
    }
    async fn create_elicitation(
        &self,
        _: rmcp::model::ElicitRequestParams,
        _: rmcp::service::RequestContext<RoleClient>,
    ) -> Result<rmcp::model::ElicitResult, rmcp::ErrorData> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        let yes = self.confirms == Some(true);
        let mut answer = rmcp::model::ElicitResult::new(if yes {
            rmcp::model::ElicitationAction::Accept
        } else {
            rmcp::model::ElicitationAction::Decline
        });
        if yes {
            answer.content = Some(json!({"confirm": true}));
        }
        Ok(answer)
    }
}

async fn connect_as(person: Person, server: McpServer) -> RunningService<RoleClient, Person> {
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let running = server.serve(server_io).await.unwrap();
        running.waiting().await.unwrap();
    });
    person.serve(client_io).await.unwrap()
}
fn person(tasks: bool, confirms: Option<bool>) -> Person {
    Person {
        tasks,
        confirms,
        asked: Arc::new(AtomicUsize::new(0)),
    }
}
fn slow_server(delay_ms: u64) -> McpServer {
    let runtime = Arc::new(
        Runtime::new()
            .tool(Slow(std::time::Duration::from_millis(delay_ms)))
            .unwrap()
            .tool(Wipe)
            .unwrap(),
    );
    McpServer::new(runtime).promote_after(Some(std::time::Duration::from_millis(50)))
}

#[tokio::test]
async fn slow_calls_become_tasks_the_client_polls() {
    let client = connect_as(person(true, None), slow_server(300)).await;
    let response = client
        .call_tool_once(call("slow", json!({"name": "x"})))
        .await
        .unwrap();
    let rmcp::model::CallToolResponse::Task(created) = response else {
        panic!("a slow call becomes a task, got {response:?}")
    };
    let task_id = created.task.task_id.clone();
    let mut last = None;
    for _ in 0..100 {
        let got = client
            .get_task(rmcp::model::GetTaskParams::new(task_id.clone()))
            .await
            .unwrap();
        if got.task.task.status.is_terminal() {
            last = Some(got);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let done = last.expect("the task finishes");
    let rmcp::model::TaskPayload::Completed { result } = done.task.payload else {
        panic!("completed, got {:?}", done.task.payload)
    };
    assert_eq!(result["structuredContent"], json!({"id": 42}));

    // Fast calls answer inline, even for task-capable clients.
    let fast = connect_as(person(true, None), slow_server(1)).await;
    let inline = fast
        .call_tool_once(call("slow", json!({"name": "x"})))
        .await
        .unwrap();
    assert!(matches!(inline, rmcp::model::CallToolResponse::Complete(_)));
}

#[tokio::test]
async fn tasks_can_be_cancelled_and_older_clients_wait_inline() {
    let client = connect_as(person(true, None), slow_server(5_000)).await;
    let rmcp::model::CallToolResponse::Task(created) = client
        .call_tool_once(call("slow", json!({"name": "x"})))
        .await
        .unwrap()
    else {
        panic!("a task")
    };
    let id = created.task.task_id.clone();
    client
        .cancel_task(rmcp::model::CancelTaskParams::new(id.clone()))
        .await
        .unwrap();
    let mut status = None;
    for _ in 0..100 {
        let got = client
            .get_task(rmcp::model::GetTaskParams::new(id.clone()))
            .await
            .unwrap();
        if got.task.task.status.is_terminal() {
            status = Some(got.task.task.status);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(status, Some(rmcp::model::TaskStatus::Cancelled));

    // A client that does not support tasks gets its answer inline, however long.
    let plain = connect_as(person(false, None), slow_server(200)).await;
    let result = plain
        .call_tool(call("slow", json!({"name": "x"})))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(false));
}

#[tokio::test]
async fn confirmation_is_asked_of_the_person_behind_the_client() {
    let yes = person(false, Some(true));
    let client = connect_as(yes.clone(), slow_server(1)).await;
    let result = client
        .call_tool(call("wipe", json!({"name": "x"})))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(false), "{result:?}");
    assert_eq!(yes.asked.load(Ordering::SeqCst), 1);

    let no = person(false, Some(false));
    let client = connect_as(no.clone(), slow_server(1)).await;
    let result = client
        .call_tool(call("wipe", json!({"name": "x"})))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let meta = result.meta.unwrap();
    assert_eq!(meta.0["carmy/error"]["code"], "CONFIRMATION_REQUIRED");
    assert_eq!(no.asked.load(Ordering::SeqCst), 1);

    // A client that cannot ask, or a server told not to, keeps today's error.
    let mute = connect_as(person(false, None), slow_server(1)).await;
    let result = mute
        .call_tool(call("wipe", json!({"name": "x"})))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let yes = person(false, Some(true));
    let client = connect_as(yes.clone(), slow_server(1).confirm_by_elicitation(false)).await;
    let result = client
        .call_tool(call("wipe", json!({"name": "x"})))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert_eq!(yes.asked.load(Ordering::SeqCst), 0, "never asked");
}
