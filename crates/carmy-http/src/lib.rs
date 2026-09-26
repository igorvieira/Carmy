//! HTTP adapter over Carmy's transport-independent runtime.
//!
//! Routes: `GET /.well-known/agent`, `GET /agent/tools`, `POST /agent/execute`
//! (JSON, or Server-Sent Events with `Accept: text/event-stream`).
//! Wire DTOs live here; runtime types never serialize directly onto the wire.
//! [`ServerOptions`] holds the connection limits, timeouts and browser protections.
mod hardening;
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, FromRequestParts, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode, header, request::Parts},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use carmy_core::*;
use carmy_runtime::{Runtime, execution_request};
use futures_util::StreamExt;
pub use hardening::{Any, CorsLayer, ServerOptions, harden, serve_listener};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{convert::Infallible, sync::Arc};

pub const DISCOVERY_PATH: &str = "/.well-known/agent";
pub const TOOLS_PATH: &str = "/agent/tools";
pub const EXECUTE_PATH: &str = "/agent/execute";
const BODY_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteDto {
    pub tool: String,
    #[serde(default = "empty_object")]
    pub arguments: Value,
    /// Stable identity for retries; repeated requests replay the first result.
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
    pub agent: AgentHints,
}
#[derive(Debug, Default, Serialize)]
pub struct AgentHints {
    /// The client may reuse this result for identical arguments.
    pub cacheable: bool,
    pub next_actions: Vec<String>,
}
impl ResultDto {
    /// `reusable` states whether the tool is idempotent and free of writes.
    pub fn new(result: ExecutionResult, reusable: bool) -> Self {
        let status = result.status.as_str();
        let (data, error) = match result.outcome {
            Ok(data) => (Some(data), None),
            Err(e) => (None, Some(e)),
        };
        Self {
            execution_id: result.execution_id,
            status,
            agent: AgentHints {
                cacheable: reusable && data.is_some(),
                next_actions: Vec::new(),
            },
            data,
            error,
        }
    }
}
fn reusable(tool: Option<&ToolMetadata>) -> bool {
    tool.is_some_and(|t| t.idempotent && matches!(t.effect, Effect::None | Effect::Read))
}
#[derive(Serialize)]
struct ToolDto<'a> {
    name: &'a str,
    description: &'a str,
    input_schema: &'a Value,
    output_schema: &'a Value,
    effect: Effect,
    idempotent: bool,
    parallel_safe: bool,
    confirmation: Confirmation,
}
impl<'a> From<&'a ToolMetadata> for ToolDto<'a> {
    fn from(m: &'a ToolMetadata) -> Self {
        Self {
            name: &m.name,
            description: &m.description,
            input_schema: &m.input_schema,
            output_schema: &m.output_schema,
            effect: m.effect,
            idempotent: m.idempotent,
            parallel_safe: m.parallel_safe,
            confirmation: m.confirmation,
        }
    }
}
/// Pre-serialized, content-addressed discovery document served with ETag revalidation.
#[derive(Clone)]
struct Document {
    body: Arc<str>,
    etag: Arc<str>,
}
impl Document {
    fn new(value: Value) -> Self {
        let body = value.to_string();
        let etag = format!("\"{:x}\"", Sha256::digest(body.as_bytes()));
        Self {
            body: body.into(),
            etag: etag.into(),
        }
    }
    fn response(&self, headers: &HeaderMap) -> Response {
        let matches = headers
            .get(header::IF_NONE_MATCH)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|s| {
                s.split(',').any(|v| {
                    let v = v.trim();
                    v == "*" || v.trim_start_matches("W/") == &*self.etag
                })
            });
        let mut response = if matches {
            StatusCode::NOT_MODIFIED.into_response()
        } else {
            self.body.to_string().into_response()
        };
        let headers = response.headers_mut();
        headers.insert(header::ETAG, self.etag.parse().unwrap());
        headers.insert(
            header::CACHE_CONTROL,
            "private, max-age=60".parse().unwrap(),
        );
        headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
        response
    }
}
#[derive(Clone)]
struct HttpState {
    runtime: Arc<Runtime>,
    discovery: Document,
    tools: Document,
}
/// Build the agent router. Host middleware may insert `Extension<AgentContext>` after
/// authentication; request bodies can never set context. Apply authentication to the
/// entire router if tool names and schemas are private.
pub fn router(runtime: Arc<Runtime>, server: impl Into<String>) -> Router {
    let metadata = runtime.tools();
    let tools: Vec<ToolDto> = metadata.iter().map(ToolDto::from).collect();
    let tools = Document::new(json!({ "tools": tools }));
    // `tools_version` lets agents skip refetching an unchanged catalog.
    let discovery = Document::new(json!({
        "protocol": "carmy/1",
        "server": server.into(),
        "capabilities": ["tools", "streaming", "idempotency"],
        "tools_url": TOOLS_PATH,
        "tools_version": tools.etag.trim_matches('"'),
        "execute_url": EXECUTE_PATH,
    }));
    let state = HttpState {
        runtime,
        discovery,
        tools,
    };
    Router::new()
        .route(DISCOVERY_PATH, get(discovery_document))
        .route(TOOLS_PATH, get(list_tools))
        .route(EXECUTE_PATH, post(execute))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(state)
}
async fn discovery_document(State(state): State<HttpState>, headers: HeaderMap) -> Response {
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
    // A host-provided context gets a child token, so cancelling one execution cannot
    // cancel a session. Without one, the request's fresh token is already independent.
    if let Some(Extension(mut ctx)) = context {
        ctx.cancellation = ctx.cancellation.child_token();
        request.context = ctx;
    }
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
/// Whether the client asked for SSE. Reads the `Accept` header in place instead of
/// cloning the whole header map, as the `HeaderMap` extractor would.
struct WantsStream(bool);
impl<S: Send + Sync> FromRequestParts<S> for WantsStream {
    type Rejection = Infallible;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Infallible> {
        let accept = parts.headers.get(header::ACCEPT);
        Ok(Self(
            accept
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.contains("text/event-stream")),
        ))
    }
}
/// Dropping this handler or its event stream (client disconnect) cancels the execution's
/// token; the runtime keeps the idempotency reservation so a retry reports uncertainty.
async fn execute(
    State(state): State<HttpState>,
    context: Option<Extension<AgentContext>>,
    WantsStream(stream): WantsStream,
    payload: Result<Json<ExecuteDto>, JsonRejection>,
) -> Response {
    let request = match decode(payload, context) {
        Ok(r) => r,
        Err(response) => return *response,
    };
    let reusable = reusable(state.runtime.metadata(&request.tool));
    if stream {
        let events = state
            .runtime
            .execute_stream(request)
            .map(move |event| Ok::<_, Infallible>(sse_event(event, reusable)));
        return Sse::new(events)
            .keep_alive(KeepAlive::default())
            .into_response();
    }
    let dto = ResultDto::new(state.runtime.execute(request).await, reusable);
    let status = status_of(dto.error.as_ref());
    (status, [(header::CACHE_CONTROL, "no-store")], Json(dto)).into_response()
}
/// SSE is only the wire encoding of runtime events. `execution.completed` carries the
/// same `ResultDto` a JSON response would, plus `replayed`.
fn sse_event(event: ExecutionEvent, reusable: bool) -> Event {
    let name = event.name();
    let data = match event {
        ExecutionEvent::ExecutionStarted { execution_id, tool }
        | ExecutionEvent::ToolStarted { execution_id, tool } => {
            json!({ "execution_id": execution_id, "tool": tool })
        }
        ExecutionEvent::ToolCompleted {
            execution_id,
            tool,
            duration_ms,
            ok,
        } => json!({
            "execution_id": execution_id,
            "tool": tool,
            "duration_ms": duration_ms,
            "ok": ok,
        }),
        ExecutionEvent::ExecutionCompleted { result, replayed } => {
            let mut data =
                serde_json::to_value(ResultDto::new(result, reusable)).expect("DTO serializes");
            data["replayed"] = replayed.into();
            data
        }
    };
    Event::default().event(name).data(data.to_string())
}
/// Bind `address` and serve [`router`] until the process stops.
pub async fn serve(
    runtime: Arc<Runtime>,
    address: impl tokio::net::ToSocketAddrs,
    server: impl Into<String>,
) -> std::io::Result<()> {
    serve_with(runtime, address, server, &ServerOptions::default()).await
}

/// [`router`] plus the per-request protections in `options`.
pub fn router_with(
    runtime: Arc<Runtime>,
    server: impl Into<String>,
    options: &ServerOptions,
) -> Router {
    harden(router(runtime, server), options)
}

/// [`serve`] with explicit [`ServerOptions`]: timeouts, a connection limit, and the
/// optional security headers and CORS.
pub async fn serve_with(
    runtime: Arc<Runtime>,
    address: impl tokio::net::ToSocketAddrs,
    server: impl Into<String>,
    options: &ServerOptions,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(address).await?;
    serve_listener(listener, router_with(runtime, server, options), options).await
}
