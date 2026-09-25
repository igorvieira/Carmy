//! Transport-independent execution and policy enforcement.
use carmy_core::*;
use serde_json::Value;
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};
use tokio::sync::Mutex;

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
        }
    }
    pub fn policy(mut self, policy: impl ExecutionPolicy + 'static) -> Self {
        self.policies.push(Arc::new(policy));
        self
    }
    /// Registration is fallible: names and schemas must be valid and unique.
    pub fn tool<T: Tool>(mut self, tool: T) -> AgentResult<Self> {
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
        Ok(self)
    }
    pub fn tools(&self) -> Vec<ToolMetadata> {
        self.tools.values().map(|t| t.metadata.clone()).collect()
    }
    pub async fn execute(&self, mut request: ExecutionRequest) -> ExecutionResult {
        request.context.execution_id = request.execution_id.clone();
        request.context.request_id = request.request_id.clone();
        self.run(request).await
    }
    async fn run(&self, request: ExecutionRequest) -> ExecutionResult {
        let id = request.execution_id.clone();
        let outcome = async {
            let registered = self
                .tools
                .get(&request.tool)
                .ok_or_else(|| error("TOOL_NOT_FOUND", "Unknown tool", ErrorCategory::NotFound))?;
            for policy in &self.policies {
                policy.check(&request.context, &registered.metadata)?;
            }
            if !registered.input.is_valid(&request.arguments) {
                return Err(error(
                    "INVALID_ARGUMENTS",
                    "Arguments do not match the input schema",
                    ErrorCategory::Validation,
                ));
            }
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
        }
        .await;
        result(id, outcome)
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
