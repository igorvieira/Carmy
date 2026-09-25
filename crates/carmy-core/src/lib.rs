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
}
impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for AgentError {}

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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Completed,
    Failed,
    Cancelled,
    TimedOut,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub execution_id: String,
    pub status: ExecutionStatus,
    pub outcome: AgentResult<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionEvent {
    pub execution_id: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ExecutionResult>,
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
        assert_eq!(
            serde_json::to_value(Effect::ExternalWrite).unwrap(),
            "external_write"
        );
    }
    #[test]
    fn cancellation_is_shared() {
        let ctx = AgentContext::default();
        let cloned = ctx.clone();
        ctx.cancellation.cancel();
        assert!(cloned.cancellation.is_cancelled());
    }
}
