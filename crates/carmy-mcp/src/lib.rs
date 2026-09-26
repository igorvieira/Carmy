//! Model Context Protocol adapter over Carmy's runtime, built on the official `rmcp` SDK.
//!
//! Supported in v0.1: `initialize`, `ping`, `tools/list` and `tools/call` over any
//! `rmcp` transport (stdio helper included), plus request cancellation
//! (`notifications/cancelled`). Resources, prompts, sampling, elicitation,
//! progress notifications and tasks are not implemented.
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
use carmy_core::*;
use carmy_runtime::{Runtime, execution_request};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        JsonObject, ListToolsResult, MetaObject, PaginatedRequestParams, ServerCapabilities,
        ServerConfig, Tool as McpTool, ToolAnnotations,
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
}
impl McpServer {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        let tools = runtime.tools().iter().map(tool).collect();
        Self {
            runtime,
            context: AgentContext::default(),
            tools,
            name: "carmy".into(),
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
fn to_mcp(result: ExecutionResult) -> CallToolResult {
    let mut meta = JsonObject::new();
    meta.insert("carmy/status".into(), result.status.as_str().into());
    meta.insert("carmy/execution_id".into(), result.execution_id.into());
    let call = match result.outcome {
        Ok(value @ Value::Object(_)) => CallToolResult::structured(value),
        Ok(value) => CallToolResult::success(vec![ContentBlock::text(value.to_string())]),
        Err(error) => {
            let error = serde_json::to_value(error).expect("AgentError serializes");
            let text = json!({ "error": &error }).to_string();
            meta.insert("carmy/error".into(), error);
            CallToolResult::error(vec![ContentBlock::text(text)])
        }
    };
    call.with_meta(Some(MetaObject(meta)))
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
        let arguments = Value::Object(params.arguments.unwrap_or_default());
        let mut request = execution_request(params.name, arguments);
        request.request_id = request_id;
        request.context = self.context.clone();
        // MCP `notifications/cancelled` cancels this execution only. A child token, so
        // the runtime's own cancellation (deadline, drop) never marks the MCP request
        // itself cancelled, which would suppress the response.
        request.context.cancellation = ctx.ct.child_token();
        let result = self.runtime.execute(request).await;
        // An unknown tool is a protocol error, not a tool result.
        if let Err(error) = &result.outcome
            && error.code == "TOOL_NOT_FOUND"
        {
            return Err(McpError::invalid_params(
                "Unknown tool",
                Some(json!({ "error": error })),
            ));
        }
        Ok(to_mcp(result).into())
    }
}
