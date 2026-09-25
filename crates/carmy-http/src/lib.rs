//! HTTP DTOs and routes over Carmy's transport-independent runtime.
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use carmy_core::*;
use carmy_runtime::{Runtime, execution_request};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteDto {
    pub tool: String,
    #[serde(default = "empty_object")]
    pub arguments: Value,
    pub request_id: Option<String>,
}
fn empty_object() -> Value {
    json!({})
}
#[derive(Debug, Serialize)]
pub struct ResultDto {
    pub execution_id: String,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AgentError>,
    #[serde(rename = "_agent")]
    pub agent: Value,
}
impl ResultDto {
    /// Successful results of side-effect-free, idempotent tools may be reused by clients.
    pub fn new(result: ExecutionResult, tool: Option<&ToolMetadata>) -> Self {
        let cacheable = result.outcome.is_ok()
            && tool
                .is_some_and(|t| t.idempotent && matches!(t.effect, Effect::None | Effect::Read));
        let mut dto = Self::from(result);
        dto.agent = json!({"cacheable":cacheable,"next_actions":[]});
        dto
    }
}
impl From<ExecutionResult> for ResultDto {
    fn from(result: ExecutionResult) -> Self {
        let status = match result.status {
            ExecutionStatus::Completed => "completed",
            ExecutionStatus::Failed => "failed",
            ExecutionStatus::Cancelled => "cancelled",
            ExecutionStatus::TimedOut => "timed_out",
        };
        let (data, error) = match result.outcome {
            Ok(data) => (Some(data), None),
            Err(e) => (None, Some(e)),
        };
        Self {
            execution_id: result.execution_id,
            status,
            data,
            error,
            agent: json!({"cacheable":false,"next_actions":[]}),
        }
    }
}
#[derive(Clone)]
struct Document {
    body: String,
    etag: String,
}
impl Document {
    fn new(value: Value) -> Self {
        let body = value.to_string();
        let etag = format!("\"{:x}\"", Sha256::digest(body.as_bytes()));
        Self { body, etag }
    }
    fn response(&self, headers: &HeaderMap) -> Response {
        let matches = headers
            .get(header::IF_NONE_MATCH)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|s| {
                s.split(',')
                    .any(|v| v.trim().trim_start_matches("W/") == self.etag || v.trim() == "*")
            });
        let mut response = if matches {
            StatusCode::NOT_MODIFIED.into_response()
        } else {
            self.body.clone().into_response()
        };
        response
            .headers_mut()
            .insert(header::ETAG, self.etag.parse().unwrap());
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            "private, max-age=60".parse().unwrap(),
        );
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
        response
    }
}
#[derive(Clone)]
struct HttpState {
    runtime: Arc<Runtime>,
    discovery: Document,
    tools: Document,
}
/// Host middleware may insert `Extension<AgentContext>` after authentication.
/// Apply authentication to the entire router if tool names/schemas are private.
pub fn router(runtime: Arc<Runtime>, server: impl Into<String>) -> Router {
    let tools: Vec<_> = runtime.tools().into_iter().map(|m| json!({
        "name":m.name,"description":m.description,"input_schema":m.input_schema,"output_schema":m.output_schema,
        "effect":m.effect,"idempotent":m.idempotent,"parallel_safe":m.parallel_safe,"confirmation":m.confirmation
    })).collect();
    let state = HttpState {
        runtime,
        discovery: Document::new(
            json!({"protocol":"carmy/1","server":server.into(),"capabilities":["tools","idempotency"],"tools_url":"/agent/tools","execute_url":"/agent/execute"}),
        ),
        tools: Document::new(json!({"tools":tools})),
    };
    Router::new()
        .route("/.well-known/agent", get(discovery))
        .route("/agent/tools", get(list_tools))
        .route("/agent/execute", post(execute))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .with_state(state)
}
async fn discovery(State(state): State<HttpState>, headers: HeaderMap) -> Response {
    state.discovery.response(&headers)
}
async fn list_tools(State(state): State<HttpState>, headers: HeaderMap) -> Response {
    state.tools.response(&headers)
}
fn decode(
    payload: Result<Json<ExecuteDto>, JsonRejection>,
    context: Option<Extension<AgentContext>>,
) -> Result<ExecutionRequest, Box<Response>> {
    let Json(dto) = payload.map_err(|e| Box::new(rejection(&e)))?;
    let mut request = execution_request(dto.tool, dto.arguments);
    request.request_id = dto.request_id;
    if let Some(Extension(ctx)) = context {
        request.context = ctx;
    }
    // Each request gets a child token, so cancelling one execution cannot cancel a session.
    request.context.cancellation = request.context.cancellation.child_token();
    Ok(request)
}
fn rejection(e: &JsonRejection) -> Response {
    let (status, mut error) = if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
        let error = AgentError::new(
            "PAYLOAD_TOO_LARGE",
            "Request body exceeds the limit",
            ErrorCategory::Capacity,
        );
        (StatusCode::PAYLOAD_TOO_LARGE, error)
    } else {
        let error = AgentError::new(
            "INVALID_REQUEST",
            "Expected a JSON execution request",
            ErrorCategory::Validation,
        );
        (StatusCode::BAD_REQUEST, error)
    };
    error.recoverable = true;
    error.details = Some(Box::new(json!({"reason": e.body_text()})));
    (status, Json(json!({ "error": error }))).into_response()
}
fn status_of(error: Option<&AgentError>) -> StatusCode {
    error.map_or(StatusCode::OK, |e| match e.category {
        ErrorCategory::Validation => StatusCode::BAD_REQUEST,
        ErrorCategory::NotFound => StatusCode::NOT_FOUND,
        ErrorCategory::Permission => StatusCode::FORBIDDEN,
        ErrorCategory::Conflict => StatusCode::CONFLICT,
        ErrorCategory::Timeout => StatusCode::GATEWAY_TIMEOUT,
        ErrorCategory::Capacity => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCategory::Cancelled => StatusCode::REQUEST_TIMEOUT,
        ErrorCategory::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    })
}
/// Dropping this handler (client disconnect) cancels the execution's token; the
/// runtime keeps the idempotency reservation so a retry reports uncertainty.
async fn execute(
    State(state): State<HttpState>,
    context: Option<Extension<AgentContext>>,
    payload: Result<Json<ExecuteDto>, JsonRejection>,
) -> Response {
    let request = match decode(payload, context) {
        Ok(r) => r,
        Err(response) => return *response,
    };
    let tool = request.tool.clone();
    let result = state.runtime.execute(request).await;
    let dto = ResultDto::new(result, state.runtime.metadata(&tool));
    let status = status_of(dto.error.as_ref());
    (status, [(header::CACHE_CONTROL, "no-store")], Json(dto)).into_response()
}
pub async fn serve(
    runtime: Arc<Runtime>,
    address: &str,
    server: impl Into<String>,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, router(runtime, server)).await
}
