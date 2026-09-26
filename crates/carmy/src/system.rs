//! Tools every Carmy app offers about itself, so an agent can follow asynchronous work
//! over any transport (HTTP, MCP, the console) with no transport-specific endpoint.
//! They go through the runtime like any tool: policies, audit and all.
//!
//! - `carmy_job`: a job's status, by the `job_id` an enqueue returned. Always present.
//! - `carmy_dead_letters` and `carmy_audit`: operator views, added by
//!   [`Carmy::operator_tools`](crate::Carmy::operator_tools).
//!
//! None of them returns a job's arguments or an execution's payload.
use crate::{
    AgentContext, AgentError, AgentResult, Confirmation, Effect, ErrorCategory, Tool, ToolMetadata,
    jobs::{Job, JobId, Jobs},
    runtime::InMemoryAudit,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// Names reserved for these tools.
pub const JOB_TOOL: &str = "carmy_job";
pub const DEAD_LETTERS_TOOL: &str = "carmy_dead_letters";
pub const AUDIT_TOOL: &str = "carmy_audit";

/// A job as agents see it: never its arguments, which may carry secrets.
#[derive(Debug, Serialize, JsonSchema)]
pub struct JobView {
    pub job_id: String,
    pub tool: String,
    /// `queued`, `running`, `succeeded`, `failed`, `dead_lettered` or `cancelled`.
    pub status: String,
    pub request_id: Option<String>,
    pub attempts: u32,
    pub max_attempts: u32,
    /// When the job runs next, or last ran (RFC 3339).
    pub run_at: String,
    /// The last error, in the usual structured form.
    pub last_error: Option<Value>,
}

impl From<Job> for JobView {
    fn from(job: Job) -> Self {
        Self {
            job_id: job.id.0,
            tool: job.request.tool,
            status: job.status.as_str().into(),
            request_id: job.request.request_id,
            attempts: job.attempts,
            max_attempts: job.max_attempts,
            run_at: job.run_at.to_rfc3339(),
            last_error: job
                .last_error
                .map(|e| serde_json::to_value(e).expect("errors serialize")),
        }
    }
}

fn read_tool<I: JsonSchema, O: JsonSchema>(name: &str, description: &str) -> ToolMetadata {
    ToolMetadata {
        name: name.into(),
        description: description.into(),
        input_schema: schemars::schema_for!(I).to_value(),
        output_schema: schemars::schema_for!(O).to_value(),
        effect: Effect::Read,
        // The answer changes as jobs run: never cacheable.
        idempotent: false,
        parallel_safe: true,
        confirmation: Confirmation::None,
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct JobInput {
    /// The `job_id` returned when the job was enqueued.
    pub job_id: String,
}

pub(crate) struct JobStatus(pub Jobs);
impl Tool for JobStatus {
    type Input = JobInput;
    type Output = JobView;
    fn metadata(&self) -> ToolMetadata {
        read_tool::<JobInput, JobView>(
            JOB_TOOL,
            "Status of a job: queued, running, succeeded, failed, dead_lettered or cancelled, with attempts and the last error. Use the job_id an enqueue returned.",
        )
    }
    async fn execute(&self, _: AgentContext, input: JobInput) -> AgentResult<JobView> {
        match self.0.get(&JobId(input.job_id.clone())).await? {
            Some(job) => Ok(job.into()),
            None => Err(AgentError::new(
                "JOB_NOT_FOUND",
                format!("no job `{}`", input.job_id),
                ErrorCategory::NotFound,
            )
            .recoverable()),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct LimitInput {
    /// How many to return, 1 to 1000. Default 20.
    #[serde(default)]
    pub limit: Option<u32>,
}
impl LimitInput {
    fn get(&self) -> usize {
        self.limit.map_or(20, |n| n.clamp(1, 1_000) as usize)
    }
}

#[derive(Serialize, JsonSchema)]
pub struct DeadLetters {
    pub jobs: Vec<JobView>,
}

pub(crate) struct DeadLettersTool(pub Jobs);
impl Tool for DeadLettersTool {
    type Input = LimitInput;
    type Output = DeadLetters;
    fn metadata(&self) -> ToolMetadata {
        read_tool::<LimitInput, DeadLetters>(
            DEAD_LETTERS_TOOL,
            "Jobs that gave up and wait for a decision: retried until max_attempts, or uncertain on a non-idempotent tool.",
        )
    }
    async fn execute(&self, _: AgentContext, input: LimitInput) -> AgentResult<DeadLetters> {
        let jobs = self.0.dead_letters(input.get()).await?;
        Ok(DeadLetters {
            jobs: jobs.into_iter().map(JobView::from).collect(),
        })
    }
}

#[derive(Serialize, JsonSchema)]
pub struct Executions {
    /// Newest first. Who ran which tool, when, and how it ended; never arguments or
    /// outputs.
    pub executions: Vec<Value>,
}

pub(crate) struct AuditTool(pub Arc<InMemoryAudit>);
impl Tool for AuditTool {
    type Input = LimitInput;
    type Output = Executions;
    fn metadata(&self) -> ToolMetadata {
        read_tool::<LimitInput, Executions>(
            AUDIT_TOOL,
            "The latest executions on this instance, newest first: principal, tool, effect, status, error code, duration, replayed.",
        )
    }
    async fn execute(&self, _: AgentContext, input: LimitInput) -> AgentResult<Executions> {
        let records = self.0.recent(input.get());
        Ok(Executions {
            executions: records
                .into_iter()
                .map(|r| serde_json::to_value(r).expect("records serialize"))
                .collect(),
        })
    }
}
