//! Transport-independent tool and execution domain. APIs are unstable in 0.1.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
};
pub use tokio_util::sync::CancellationToken;

pub type AgentResult<T> = Result<T, AgentError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    None,
    Read,
    Write,
    ExternalWrite,
    Destructive,
}
impl Effect {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Read => "read",
            Self::Write => "write",
            Self::ExternalWrite => "external_write",
            Self::Destructive => "destructive",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confirmation {
    None,
    Required,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMetadata {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub effect: Effect,
    pub idempotent: bool,
    pub parallel_safe: bool,
    pub confirmation: Confirmation,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    Validation,
    NotFound,
    Permission,
    Conflict,
    Cancelled,
    Timeout,
    Internal,
    Capacity,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentError {
    pub code: String,
    pub message: String,
    pub category: ErrorCategory,
    pub recoverable: bool,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Box<Value>>,
}
impl AgentError {
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        category: ErrorCategory,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            category,
            recoverable: false,
            retryable: false,
            retry_after: None,
            suggested_action: None,
            details: None,
        }
    }
    /// The agent can fix the request (e.g. change arguments) and continue.
    pub fn recoverable(mut self) -> Self {
        self.recoverable = true;
        self
    }
    /// Retrying the same request may succeed, optionally after `seconds`.
    pub fn retryable(mut self, after_seconds: Option<u64>) -> Self {
        self.retryable = true;
        self.recoverable = true;
        self.retry_after = after_seconds;
        self
    }
    /// Name of a tool the agent should consider calling next.
    pub fn suggest(mut self, action: impl Into<String>) -> Self {
        self.suggested_action = Some(action.into());
        self
    }
    pub fn details(mut self, details: Value) -> Self {
        self.details = Some(Box::new(details));
        self
    }
}
impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for AgentError {}

/// Input of tools that take no arguments: accepts only `{}`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoInput {}

/// Trusted framework context, constructed by the host, never from untrusted arguments.
#[derive(Debug, Clone, Default)]
pub struct AgentContext {
    pub execution_id: String,
    pub request_id: Option<String>,
    pub session: Option<String>,
    pub principal: Option<String>,
    pub permissions: BTreeSet<String>,
    pub metadata: BTreeMap<String, Value>,
    pub cancellation: CancellationToken,
    /// Report progress or partial results while the tool runs.
    pub progress: Progress,
}

/// One progress report from a running tool.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProgressUpdate {
    /// How far along, in the tool's own unit (items, bytes, percent).
    pub progress: f64,
    /// The amount `progress` goes up to, when known.
    pub total: Option<f64>,
    /// A short human-readable status.
    pub message: Option<String>,
    /// A piece of the result that is already useful.
    pub partial: Option<Value>,
}

/// Where a tool reports progress. Set by the runtime when someone listens (a stream,
/// SSE, the console, an MCP progress token); otherwise every call is a no-op, so tools
/// can report unconditionally.
///
/// ```
/// # use carmy_core::AgentContext;
/// # let ctx = AgentContext::default();
/// ctx.progress.report(3.0, Some(10.0), "3 of 10 pages");
/// ```
#[derive(Clone, Default)]
pub struct Progress {
    sink: Option<std::sync::Arc<dyn Fn(ProgressUpdate) + Send + Sync>>,
}

impl Progress {
    /// A progress handle that hands every update to `sink`.
    pub fn new(sink: impl Fn(ProgressUpdate) + Send + Sync + 'static) -> Self {
        Self {
            sink: Some(std::sync::Arc::new(sink)),
        }
    }
    /// Whether anyone receives the updates; skip expensive reports when not.
    pub fn is_listening(&self) -> bool {
        self.sink.is_some()
    }
    /// Report how far along the tool is. An empty `message` sends none.
    pub fn report(&self, progress: f64, total: Option<f64>, message: impl Into<String>) {
        self.send(|| {
            let message: String = message.into();
            ProgressUpdate {
                progress,
                total,
                message: (!message.is_empty()).then_some(message),
                partial: None,
            }
        });
    }
    /// Hand over a piece of the result before the tool finishes.
    pub fn partial(&self, value: impl Serialize) {
        self.send(|| ProgressUpdate {
            partial: serde_json::to_value(value).ok(),
            ..ProgressUpdate::default()
        });
    }
    /// Send a full update.
    pub fn update(&self, update: ProgressUpdate) {
        self.send(|| update);
    }
    fn send(&self, update: impl FnOnce() -> ProgressUpdate) {
        if let Some(sink) = &self.sink {
            sink(update());
        }
    }
}

impl std::fmt::Debug for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Progress")
            .field("listening", &self.is_listening())
            .finish()
    }
}
/// Implement on application structs to inject dependencies explicitly.
pub trait Tool: Send + Sync + 'static {
    type Input: DeserializeOwned + JsonSchema + Send + 'static;
    type Output: Serialize + JsonSchema + Send + 'static;
    fn metadata(&self) -> ToolMetadata;
    fn execute(
        &self,
        ctx: AgentContext,
        input: Self::Input,
    ) -> impl Future<Output = AgentResult<Self::Output>> + Send;
}
#[derive(Debug, Clone)]
pub struct ExecutionRequest {
    pub execution_id: String,
    pub request_id: Option<String>,
    pub tool: String,
    pub arguments: Value,
    pub context: AgentContext,
    pub metadata: BTreeMap<String, Value>,
}
impl ExecutionRequest {
    /// The idempotency identity of this request: repeats replay the recorded result.
    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Completed,
    Failed,
    Cancelled,
    TimedOut,
}
impl ExecutionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub execution_id: String,
    pub status: ExecutionStatus,
    pub outcome: AgentResult<Value>,
}
/// Lifecycle events emitted by the runtime, independent of any wire encoding.
/// `ToolStarted`/`ToolCompleted` are absent when a result is replayed or the
/// request is rejected before invocation; `ToolProgress` comes between them, as
/// often as the tool reports. `ExecutionCompleted` is always last. New variants may
/// be added, so matches need a wildcard arm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ExecutionEvent {
    ExecutionStarted {
        execution_id: String,
        tool: String,
    },
    ToolStarted {
        execution_id: String,
        tool: String,
    },
    ToolProgress {
        execution_id: String,
        tool: String,
        progress: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        total: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        partial: Option<Value>,
    },
    ToolCompleted {
        execution_id: String,
        tool: String,
        duration_ms: u64,
        ok: bool,
    },
    ExecutionCompleted {
        result: ExecutionResult,
        replayed: bool,
    },
}
impl ExecutionEvent {
    /// Stable dotted name, e.g. `execution.started`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::ExecutionStarted { .. } => "execution.started",
            Self::ToolStarted { .. } => "tool.started",
            Self::ToolProgress { .. } => "tool.progress",
            Self::ToolCompleted { .. } => "tool.completed",
            Self::ExecutionCompleted { .. } => "execution.completed",
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn errors_and_effects_are_machine_readable() {
        let e = AgentError::new("DENIED", "Permission required", ErrorCategory::Permission);
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "DENIED");
        assert_eq!(v["retryable"], false);
        assert!(v.get("details").is_none());
        for effect in [
            Effect::None,
            Effect::Read,
            Effect::Write,
            Effect::ExternalWrite,
            Effect::Destructive,
        ] {
            assert_eq!(serde_json::to_value(effect).unwrap(), effect.as_str());
        }
        for status in [ExecutionStatus::Completed, ExecutionStatus::TimedOut] {
            assert_eq!(serde_json::to_value(status).unwrap(), status.as_str());
        }
    }
    #[test]
    fn error_builders_set_machine_semantics() {
        let e = AgentError::new("USER_NOT_FOUND", "No such user", ErrorCategory::NotFound)
            .recoverable()
            .suggest("search_users");
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["recoverable"], true);
        assert_eq!(v["retryable"], false);
        assert_eq!(v["suggested_action"], "search_users");
        let e = AgentError::new("BUSY", "Try later", ErrorCategory::Capacity).retryable(Some(5));
        assert!(e.recoverable && e.retryable);
        assert_eq!(e.retry_after, Some(5));
    }
    #[test]
    fn cancellation_is_shared() {
        let ctx = AgentContext::default();
        let cloned = ctx.clone();
        ctx.cancellation.cancel();
        assert!(cloned.cancellation.is_cancelled());
    }
}
