//! Transport-independent execution and policy enforcement.
pub mod idempotency;
use carmy_core::*;
use futures_util::{FutureExt, Stream};
pub use idempotency::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};
use std::{
    panic::AssertUnwindSafe,
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, mpsc};
use tracing::{Instrument, field};

type ToolFuture<'a> = Pin<Box<dyn Future<Output = AgentResult<Value>> + Send + 'a>>;
trait ErasedTool: Send + Sync {
    fn invoke(&self, ctx: AgentContext, arguments: Value) -> ToolFuture<'_>;
}
struct Typed<T>(T);
impl<T: Tool> ErasedTool for Typed<T> {
    fn invoke(&self, ctx: AgentContext, arguments: Value) -> ToolFuture<'_> {
        Box::pin(async move {
            let input = serde_json::from_value(arguments).map_err(|_| {
                error(
                    "INVALID_ARGUMENTS",
                    "Arguments do not match the input type",
                    ErrorCategory::Validation,
                )
            })?;
            let output = self.0.execute(ctx, input).await?;
            serde_json::to_value(output).map_err(|_| {
                error(
                    "INVALID_OUTPUT",
                    "Tool output could not be serialized",
                    ErrorCategory::Internal,
                )
            })
        })
    }
}
struct Registered {
    tool: Box<dyn ErasedTool>,
    metadata: ToolMetadata,
    input: jsonschema::Validator,
    output: jsonschema::Validator,
    serial: Mutex<()>,
}
/// Trusted host hooks for permissions, effect restrictions, confirmation and quotas.
/// Hooks run even on cached requests. Never accept permissions from tool arguments.
pub trait ExecutionPolicy: Send + Sync {
    fn check(&self, context: &AgentContext, tool: &ToolMetadata) -> AgentResult<()>;
}
/// Default permits tools except those requiring an explicit host confirmation grant.
pub struct SafePolicy;
impl ExecutionPolicy for SafePolicy {
    fn check(&self, ctx: &AgentContext, tool: &ToolMetadata) -> AgentResult<()> {
        if (tool.confirmation == Confirmation::Required || tool.effect == Effect::Destructive)
            && !ctx.permissions.contains(&format!("confirm:{}", tool.name))
        {
            return Err(error(
                "CONFIRMATION_REQUIRED",
                "Trusted confirmation is required",
                ErrorCategory::Permission,
            ));
        }
        Ok(())
    }
}
/// A simple optional permission hook. Combine with additional host policies.
pub struct RequireToolPermission;
impl ExecutionPolicy for RequireToolPermission {
    fn check(&self, ctx: &AgentContext, tool: &ToolMetadata) -> AgentResult<()> {
        if ctx.permissions.contains(&format!("tool:{}", tool.name)) {
            Ok(())
        } else {
            Err(error(
                "FORBIDDEN",
                "Tool permission is required",
                ErrorCategory::Permission,
            ))
        }
    }
}
pub struct Runtime {
    tools: BTreeMap<String, Registered>,
    policies: Vec<Arc<dyn ExecutionPolicy>>,
    store: Arc<dyn IdempotencyStore>,
    timeout: Duration,
}
impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}
impl Runtime {
    pub fn new() -> Self {
        Self {
            tools: BTreeMap::new(),
            policies: vec![Arc::new(SafePolicy)],
            store: Arc::new(InMemoryIdempotencyStore::default()),
            timeout: Duration::from_secs(30),
        }
    }
    pub fn policy(mut self, policy: impl ExecutionPolicy + 'static) -> Self {
        self.policies.push(Arc::new(policy));
        self
    }
    /// Registration is fallible: names and schemas must be valid and unique.
    pub fn tool<T: Tool>(mut self, tool: T) -> AgentResult<Self> {
        self.register(tool)?;
        Ok(self)
    }
    /// Like [`Runtime::tool`], keeping the runtime usable when registration fails.
    pub fn register<T: Tool>(&mut self, tool: T) -> AgentResult<()> {
        let metadata = tool.metadata();
        if metadata.name.is_empty()
            || metadata.name.len() > 128
            || !metadata
                .name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-.".contains(&c))
        {
            return Err(error(
                "INVALID_TOOL_NAME",
                "Use 1–128 ASCII letters, digits, _, - or .",
                ErrorCategory::Validation,
            ));
        }
        if self.tools.contains_key(&metadata.name) {
            return Err(error(
                "DUPLICATE_TOOL",
                "Tool name already registered",
                ErrorCategory::Conflict,
            ));
        }
        let input = jsonschema::validator_for(&metadata.input_schema).map_err(|_| {
            error(
                "INVALID_SCHEMA",
                "Invalid input schema",
                ErrorCategory::Validation,
            )
        })?;
        let output = jsonschema::validator_for(&metadata.output_schema).map_err(|_| {
            error(
                "INVALID_SCHEMA",
                "Invalid output schema",
                ErrorCategory::Validation,
            )
        })?;
        self.tools.insert(
            metadata.name.clone(),
            Registered {
                tool: Box::new(Typed(tool)),
                metadata,
                input,
                output,
                serial: Mutex::new(()),
            },
        );
        Ok(())
    }
    pub fn tools(&self) -> Vec<ToolMetadata> {
        self.tools.values().map(|t| t.metadata.clone()).collect()
    }
    pub fn metadata(&self, tool: &str) -> Option<&ToolMetadata> {
        self.tools.get(tool).map(|t| &t.metadata)
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    pub fn idempotency_store(mut self, store: Arc<dyn IdempotencyStore>) -> Self {
        self.store = store;
        self
    }
    pub async fn execute(&self, request: ExecutionRequest) -> ExecutionResult {
        self.execute_with(request, None).await
    }
    /// Execute while yielding lifecycle events. The stream drives the execution itself:
    /// dropping it before `ExecutionCompleted` cancels the execution exactly like
    /// dropping the `execute` future does.
    pub fn execute_stream(self: &Arc<Self>, request: ExecutionRequest) -> ExecutionStream {
        let (sender, events) = mpsc::unbounded_channel();
        let runtime = self.clone();
        ExecutionStream {
            work: Some(Box::pin(async move {
                runtime.execute_with(request, Some(sender)).await;
            })),
            events,
        }
    }
    async fn execute_with(
        &self,
        mut request: ExecutionRequest,
        events: Option<Sender>,
    ) -> ExecutionResult {
        request.context.execution_id = request.execution_id.clone();
        request.context.request_id = request.request_id.clone();
        let cancellation = request.context.cancellation.clone();
        let cancel_on_drop = cancellation.clone().drop_guard();
        let id = request.execution_id.clone();
        let emit = |event| {
            if let Some(sender) = &events {
                let _ = sender.send(event);
            }
        };
        // Arguments and outputs are never recorded: they may carry secrets.
        let span = tracing::info_span!(
            "carmy.execution",
            execution_id = %id,
            request_id = request.request_id.as_deref(),
            tool = %request.tool,
            effect = self.metadata(&request.tool).map(|m| m.effect.as_str()),
            status = field::Empty,
            duration_ms = field::Empty,
            replayed = field::Empty,
            error_code = field::Empty,
        );
        let started = Instant::now();
        emit(ExecutionEvent::ExecutionStarted {
            execution_id: id.clone(),
            tool: request.tool.clone(),
        });
        let (response, replayed) = match self.run(request, &emit).instrument(span.clone()).await {
            Ok(done) => done,
            Err(e) => (result(id, Err(e)), false),
        };
        span.record("status", response.status.as_str());
        span.record("duration_ms", started.elapsed().as_millis() as u64);
        span.record("replayed", replayed);
        match &response.outcome {
            Ok(_) => tracing::info!(parent: &span, "execution finished"),
            Err(e) => {
                span.record("error_code", e.code.as_str());
                tracing::warn!(parent: &span, "execution failed");
            }
        }
        emit(ExecutionEvent::ExecutionCompleted {
            result: response.clone(),
            replayed,
        });
        cancel_on_drop.disarm();
        response
    }
    async fn run(
        &self,
        request: ExecutionRequest,
        emit: &(dyn Fn(ExecutionEvent) + Send + Sync),
    ) -> AgentResult<(ExecutionResult, bool)> {
        let registered = self
            .tools
            .get(&request.tool)
            .ok_or_else(|| error("TOOL_NOT_FOUND", "Unknown tool", ErrorCategory::NotFound))?;
        for policy in &self.policies {
            policy.check(&request.context, &registered.metadata)?;
        }
        if request.execution_id.is_empty()
            || request
                .request_id
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.len() > 256)
        {
            return Err(error(
                "INVALID_ID",
                "Execution and request IDs must be nonempty; request IDs are limited to 256 bytes",
                ErrorCategory::Validation,
            ));
        }
        if !registered.input.is_valid(&request.arguments) {
            return Err(error(
                "INVALID_ARGUMENTS",
                "Arguments do not match the input schema",
                ErrorCategory::Validation,
            ));
        }
        if request.context.cancellation.is_cancelled() {
            return Err(error(
                "CANCELLED",
                "Execution cancelled before invocation",
                ErrorCategory::Cancelled,
            ));
        }
        let key = request
            .request_id
            .as_ref()
            .map(|request_id| IdempotencyKey {
                principal: request.context.principal.clone(),
                session: request.context.session.clone(),
                request_id: request_id.clone(),
            });
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&serde_json::json!([
                    request.tool,
                    request.arguments,
                    request.metadata,
                    request.context.metadata
                ]))
                .expect("JSON values serialize")
            )
        );
        if let Some(key) = &key {
            match self.store.reserve(key, &fingerprint).await? {
                Reservation::Acquired => {}
                Reservation::Replay(result) => return Ok((result, true)),
                Reservation::Conflict => {
                    return Err(error(
                        "IDEMPOTENCY_CONFLICT",
                        "Request identity was used with different tool or input",
                        ErrorCategory::Conflict,
                    ));
                }
                Reservation::InProgress => {
                    return Err(error(
                        "EXECUTION_UNCERTAIN",
                        "Execution is running or was interrupted; reconcile before retrying",
                        ErrorCategory::Conflict,
                    ));
                }
            }
        }
        let cancellation = request.context.cancellation.clone();
        let (execution_id, tool) = (request.execution_id, request.tool);
        emit(ExecutionEvent::ToolStarted {
            execution_id: execution_id.clone(),
            tool: tool.clone(),
        });
        let started = Instant::now();
        let work = async {
            let _guard = if !registered.metadata.parallel_safe {
                Some(registered.serial.lock().await)
            } else {
                None
            };
            let value = registered
                .tool
                .invoke(request.context, request.arguments)
                .await?;
            if !registered.output.is_valid(&value) {
                return Err(error(
                    "INVALID_OUTPUT",
                    "Tool output does not match the output schema",
                    ErrorCategory::Internal,
                ));
            }
            Ok(value)
        };
        let outcome = tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(error("CANCELLED", "Execution cancelled; external effects may have committed", ErrorCategory::Cancelled)),
            _ = tokio::time::sleep(self.timeout) => { cancellation.cancel(); Err(error("TIMEOUT", "Execution timed out; external effects may have committed", ErrorCategory::Timeout)) },
            result = AssertUnwindSafe(work).catch_unwind() => result.unwrap_or_else(|_| Err(error("TOOL_PANIC", "Tool panicked; external effects may have committed", ErrorCategory::Internal))),
        };
        emit(ExecutionEvent::ToolCompleted {
            execution_id: execution_id.clone(),
            tool,
            duration_ms: started.elapsed().as_millis() as u64,
            ok: outcome.is_ok(),
        });
        let result = result(execution_id, outcome);
        if let Some(key) = &key {
            self.store.complete(key, &fingerprint, &result).await?;
        }
        Ok((result, false))
    }
}
type Sender = mpsc::UnboundedSender<ExecutionEvent>;
/// Stream of [`ExecutionEvent`]s ending with `ExecutionCompleted`.
pub struct ExecutionStream {
    work: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    events: mpsc::UnboundedReceiver<ExecutionEvent>,
}
impl Stream for ExecutionStream {
    type Item = ExecutionEvent;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<ExecutionEvent>> {
        if let Some(work) = self.work.as_mut()
            && work.as_mut().poll(cx).is_ready()
        {
            self.work = None;
        }
        self.events.poll_recv(cx)
    }
}
/// Create a request with a fresh execution ID; callers supply a stable request ID for retries.
pub fn execution_request(tool: impl Into<String>, arguments: Value) -> ExecutionRequest {
    ExecutionRequest {
        execution_id: format!("exec_{}", uuid::Uuid::new_v4()),
        request_id: None,
        tool: tool.into(),
        arguments,
        context: AgentContext::default(),
        metadata: BTreeMap::new(),
    }
}

pub(crate) fn error(code: &str, message: &str, category: ErrorCategory) -> AgentError {
    AgentError::new(code, message, category)
}
fn result(execution_id: String, outcome: AgentResult<Value>) -> ExecutionResult {
    let status = match &outcome {
        Ok(_) => ExecutionStatus::Completed,
        Err(e) if e.category == ErrorCategory::Cancelled => ExecutionStatus::Cancelled,
        Err(e) if e.category == ErrorCategory::Timeout => ExecutionStatus::TimedOut,
        Err(_) => ExecutionStatus::Failed,
    };
    ExecutionResult {
        execution_id,
        status,
        outcome,
    }
}
