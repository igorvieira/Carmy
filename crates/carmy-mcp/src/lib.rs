//! Model Context Protocol adapter over Carmy's runtime, built on the official `rmcp` SDK.
//!
//! Supported: `initialize`, `ping`, `tools/list` and `tools/call` over any `rmcp`
//! transport (stdio helper included), request cancellation (`notifications/cancelled`),
//! and progress: when a `tools/call` carries a `progressToken`, what the tool reports
//! through `ctx.progress` arrives as `notifications/progress` (partial results are not
//! part of MCP progress, so they stay on Carmy's own streams). Resources, prompts and
//! sampling are not implemented.
//!
//! Mapping:
//! - Carmy effects become MCP tool annotations (`readOnlyHint`, `destructiveHint`,
//!   `idempotentHint`, `openWorldHint`); the exact Carmy metadata is kept in the
//!   tool's `_meta` under `carmy/*` keys.
//! - Tool failures are `isError` results carrying `{"error": AgentError}` as JSON text,
//!   with the error also in `_meta["carmy/error"]`. They never set `structuredContent`,
//!   which clients validate against the output schema. An unknown tool is a JSON-RPC
//!   invalid-params error.
//! - `_meta["carmy/request_id"]` on `tools/call` is the idempotency identity.
//! - Errors carry `_meta["carmy/execution_id"]` and `_meta["carmy/status"]`; successful
//!   results do too with [`McpServer::execution_meta`]. When Carmy knows what to do
//!   next (retry, confirm, poll a job), `_meta["carmy/next_actions"]` says so.
use carmy_core::*;
use carmy_runtime::{Runtime, execution_request};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        JsonObject, ListToolsResult, MetaObject, PaginatedRequestParams, ProgressNotificationParam,
        ServerCapabilities, ServerConfig, Tool as McpTool, ToolAnnotations,
    },
    service::RequestContext,
};
use serde_json::{Value, json};
use std::sync::Arc;

/// `_meta` key carrying the idempotency identity of a `tools/call`.
pub const REQUEST_ID_META: &str = "carmy/request_id";

#[derive(Clone)]
pub struct McpServer {
    runtime: Arc<Runtime>,
    context: AgentContext,
    tools: Arc<[McpTool]>,
    name: String,
    execution_meta: bool,
}
impl McpServer {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        let tools = runtime.tools().iter().map(tool).collect();
        Self {
            runtime,
            context: AgentContext::default(),
            tools,
            name: "carmy".into(),
            execution_meta: false,
        }
    }
    /// Trusted context applied to every call on this connection, e.g. the
    /// authenticated principal or confirmation grants decided by the host.
    pub fn context(mut self, context: AgentContext) -> Self {
        self.context = context;
        self
    }
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
    /// Also attach `_meta["carmy/execution_id"]` and `_meta["carmy/status"]` to successful
    /// results, e.g. to correlate calls with traces. Off by default: every extra object
    /// costs clients a parse on each response. Errors always carry them.
    pub fn execution_meta(mut self, enabled: bool) -> Self {
        self.execution_meta = enabled;
        self
    }
    /// MCP over Streamable HTTP, as a tower service: mount it in a router with
    /// `nest_service("/mcp", ..)`. Sessions stay in memory. `allowed_hosts` guards
    /// against DNS rebinding by checking `Host`; `None` keeps rmcp's default
    /// (localhost only). List your public hosts in production.
    ///
    /// When the request carries an `AgentContext` extension (inserted by the host's
    /// authentication middleware, as for Carmy's HTTP routes), calls use it instead
    /// of [`McpServer::context`].
    #[cfg(feature = "http")]
    pub fn http_service(
        self,
        allowed_hosts: Option<Vec<String>>,
    ) -> rmcp::transport::streamable_http_server::StreamableHttpService<
        Self,
        rmcp::transport::streamable_http_server::session::local::LocalSessionManager,
    > {
        use rmcp::transport::streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService,
        };
        let mut config = StreamableHttpServerConfig::default();
        if let Some(hosts) = allowed_hosts {
            config = config.with_allowed_hosts(hosts);
        }
        StreamableHttpService::new(move || Ok(self.clone()), Default::default(), config)
    }
    /// Serve a single client over stdin/stdout until it disconnects.
    pub async fn serve_stdio(self) -> std::io::Result<()> {
        let running = match stdio::pipes() {
            Some(pipes) => self.serve(pipes).await,
            None => self.serve(rmcp::transport::stdio()).await,
        }
        .map_err(std::io::Error::other)?;
        running.waiting().await.map_err(std::io::Error::other)?;
        Ok(())
    }
}

/// Non-blocking stdio. `tokio::io::stdin`/`stdout` run every read and write on the
/// blocking thread pool; when stdin and stdout are pipes (how MCP clients launch
/// servers), readiness-driven pipes avoid that per-message thread hand-off.
mod stdio {
    #[cfg(unix)]
    pub fn pipes() -> Option<(
        tokio::net::unix::pipe::Receiver,
        tokio::net::unix::pipe::Sender,
    )> {
        use std::os::fd::AsFd;
        use tokio::net::unix::pipe;
        // Duplicated descriptors: dropping them never closes the process's own stdio.
        let stdin = std::io::stdin().as_fd().try_clone_to_owned().ok()?;
        let stdout = std::io::stdout().as_fd().try_clone_to_owned().ok()?;
        // Fails, and falls back to blocking stdio, unless both are FIFOs.
        let receiver = pipe::Receiver::from_owned_fd(stdin).ok()?;
        let sender = pipe::Sender::from_owned_fd(stdout).ok()?;
        Some((receiver, sender))
    }
    #[cfg(not(unix))]
    pub fn pipes() -> Option<(tokio::io::Stdin, tokio::io::Stdout)> {
        None
    }
}
fn tool(m: &ToolMetadata) -> McpTool {
    let read_only = matches!(m.effect, Effect::None | Effect::Read);
    let annotations = ToolAnnotations::new()
        .read_only(read_only)
        .destructive(m.effect == Effect::Destructive)
        .idempotent(m.idempotent)
        .open_world(m.effect == Effect::ExternalWrite);
    let meta = json!({
        "carmy/effect": m.effect,
        "carmy/confirmation": m.confirmation,
        "carmy/parallel_safe": m.parallel_safe,
    });
    let mut t = McpTool::new(
        m.name.clone(),
        m.description.clone(),
        object(&m.input_schema),
    )
    .with_annotations(annotations)
    .with_meta(MetaObject(object(&meta)));
    // MCP output schemas and structured content must be JSON objects.
    if m.output_schema["type"] == "object" {
        t = t.with_raw_output_schema(Arc::new(object(&m.output_schema)));
    }
    t
}
fn object(value: &Value) -> JsonObject {
    value.as_object().cloned().unwrap_or_default()
}
fn request_id(params: &CallToolRequestParams, ctx: &RequestContext<RoleServer>) -> Option<String> {
    params
        .meta
        .as_ref()
        .and_then(|m| m.0.0.get(REQUEST_ID_META))
        .or_else(|| ctx.meta.0.0.get(REQUEST_ID_META))
        .and_then(Value::as_str)
        .map(str::to_owned)
}
/// Errors never use `structuredContent`: clients validate it against the tool's output
/// schema even when `isError` is set. The error travels as JSON text for the model and,
/// complete, in `_meta["carmy/error"]` for programs.
fn to_mcp(tool: &str, result: ExecutionResult, execution_meta: bool) -> CallToolResult {
    let ExecutionResult {
        execution_id,
        status,
        outcome,
    } = result;
    let next = next_actions(tool, outcome.as_ref());
    let has_next = !next.is_empty();
    let meta = move || {
        let mut meta = JsonObject::new();
        meta.insert("carmy/status".into(), status.as_str().into());
        meta.insert("carmy/execution_id".into(), execution_id.into());
        if !next.is_empty() {
            let next = serde_json::to_value(next).expect("actions serialize");
            meta.insert("carmy/next_actions".into(), next);
        }
        meta
    };
    match outcome {
        Ok(value) => {
            // Suggestions are useful enough to send even without `execution_meta`.
            let send_meta = execution_meta || has_next;
            let call = match value {
                value @ Value::Object(_) => CallToolResult::structured(value),
                value => CallToolResult::success(vec![ContentBlock::text(value.to_string())]),
            };
            match send_meta {
                true => call.with_meta(Some(MetaObject(meta()))),
                false => call,
            }
        }
        Err(error) => {
            let mut meta = meta();
            let error = serde_json::to_value(error).expect("AgentError serializes");
            let text = json!({ "error": &error }).to_string();
            meta.insert("carmy/error".into(), error);
            CallToolResult::error(vec![ContentBlock::text(text)]).with_meta(Some(MetaObject(meta)))
        }
    }
}
/// The `AgentContext` a host's middleware put on the HTTP request, when served over HTTP.
#[cfg(feature = "http")]
fn host_context(ctx: &RequestContext<RoleServer>) -> Option<AgentContext> {
    ctx.extensions
        .get::<http::request::Parts>()?
        .extensions
        .get::<AgentContext>()
        .cloned()
}
#[cfg(not(feature = "http"))]
fn host_context(_: &RequestContext<RoleServer>) -> Option<AgentContext> {
    None
}
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new(self.name.clone(), env!("CARGO_PKG_VERSION")),
        )
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(self.tools.to_vec()).with_ttl_ms(60_000))
    }
    async fn call_tool(
        &self,
        params: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let request_id = request_id(&params, &ctx);
        let tool = params.name.to_string();
        let arguments = Value::Object(params.arguments.unwrap_or_default());
        let mut request = execution_request(params.name, arguments);
        request.request_id = request_id;
        request.context = host_context(&ctx).unwrap_or_else(|| self.context.clone());
        // MCP `notifications/cancelled` cancels this execution only. A child token, so
        // the runtime's own cancellation (deadline, drop) never marks the MCP request
        // itself cancelled, which would suppress the response.
        request.context.cancellation = ctx.ct.child_token();
        let result = match ctx.meta.get_progress_token() {
            None => self.runtime.execute(request).await,
            // A client asking for progress gets the tool's reports as notifications.
            Some(token) => {
                use futures_util::StreamExt;
                let mut events = self.runtime.execute_stream(request);
                let mut last = None;
                while let Some(event) = events.next().await {
                    if let ExecutionEvent::ToolProgress {
                        progress,
                        total,
                        message,
                        ..
                    } = &event
                    {
                        let mut update = ProgressNotificationParam::new(token.clone(), *progress);
                        if let Some(total) = total {
                            update = update.with_total(*total);
                        }
                        if let Some(message) = message {
                            update = update.with_message(message.clone());
                        }
                        // Best effort: a client that stopped listening still gets its answer.
                        let _ = ctx.peer.notify_progress(update).await;
                    }
                    last = Some(event);
                }
                match last {
                    Some(ExecutionEvent::ExecutionCompleted { result, .. }) => result,
                    _ => unreachable!("an execution stream always ends with ExecutionCompleted"),
                }
            }
        };
        // An unknown tool is a protocol error, not a tool result.
        if let Err(error) = &result.outcome
            && error.code == "TOOL_NOT_FOUND"
        {
            return Err(McpError::invalid_params(
                "Unknown tool",
                Some(json!({ "error": error })),
            ));
        }
        Ok(to_mcp(&tool, result, self.execution_meta).into())
    }
}
