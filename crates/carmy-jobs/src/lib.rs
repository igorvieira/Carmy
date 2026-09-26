//! Tools that run later: a queue of `ExecutionRequest`s with retries, dead-lettering,
//! delays and schedules. A job is idempotent by construction, because it is an
//! execution with a `request_id`.
//!
//! The runtime still executes *now*; this crate decides *when*. Structured errors
//! drive the queue: `retryable` decides whether to try again and `retry_after` bounds
//! the backoff. Timeouts, cancellations and panics are *uncertain*, and are retried
//! only for tools that declare themselves `idempotent`; otherwise the job is
//! dead-lettered for reconciliation. APIs are unstable during 0.x.
mod store;

use carmy_core::{
    AgentContext, AgentError, AgentResult, CancellationToken, ErrorCategory, ExecutionRequest,
    ExecutionStatus,
};
use carmy_runtime::Runtime;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};
pub use store::{InMemoryJobStore, JobStore, StoreFuture};
use tokio::sync::Semaphore;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JobId(pub String);

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    DeadLettered,
    Cancelled,
}

impl JobStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::DeadLettered | Self::Cancelled
        )
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::DeadLettered => "dead_lettered",
            Self::Cancelled => "cancelled",
        }
    }
}

/// The serializable part of an [`ExecutionRequest`]: what a store keeps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobRequest {
    pub tool: String,
    pub arguments: Value,
    pub request_id: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
    pub principal: Option<String>,
    pub session: Option<String>,
    #[serde(default)]
    pub permissions: BTreeSet<String>,
    #[serde(default)]
    pub context_metadata: BTreeMap<String, Value>,
}

impl From<ExecutionRequest> for JobRequest {
    fn from(request: ExecutionRequest) -> Self {
        Self {
            tool: request.tool,
            arguments: request.arguments,
            request_id: request.request_id,
            metadata: request.metadata,
            principal: request.context.principal,
            session: request.context.session,
            permissions: request.context.permissions,
            context_metadata: request.context.metadata,
        }
    }
}

impl JobRequest {
    /// The execution for attempt `attempt`. Later attempts get a derived `request_id`,
    /// because the runtime has recorded the earlier one and would replay it.
    fn execution(&self, attempt: u32, cancellation: CancellationToken) -> ExecutionRequest {
        let mut request =
            carmy_runtime::execution_request(self.tool.clone(), self.arguments.clone());
        request.request_id = self.request_id.as_ref().map(|id| match attempt {
            0 | 1 => id.clone(),
            n => format!("{id}#{n}"),
        });
        request.metadata = self.metadata.clone();
        request.context = AgentContext {
            principal: self.principal.clone(),
            session: self.session.clone(),
            permissions: self.permissions.clone(),
            metadata: self.context_metadata.clone(),
            cancellation,
            ..AgentContext::default()
        };
        request
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: JobId,
    pub request: JobRequest,
    pub run_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub attempts: u32,
    pub max_attempts: u32,
    pub status: JobStatus,
    pub last_error: Option<AgentError>,
    pub lease_until: Option<DateTime<Utc>>,
    /// The schedule that produced this job, if any.
    pub schedule: Option<String>,
}

/// What a worker reports to the store after running a job.
#[derive(Debug, Clone, PartialEq)]
pub enum JobOutcome {
    Succeeded,
    Retry {
        at: DateTime<Utc>,
        error: AgentError,
    },
    Failed(AgentError),
    DeadLettered(AgentError),
}

/// Backoff and attempt limits.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base: Duration,
    pub cap: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base: Duration::from_secs(1),
            cap: Duration::from_secs(3600),
        }
    }
}

impl RetryPolicy {
    /// Exponential backoff for the attempt that just failed, at least `retry_after`.
    fn delay(&self, attempt: u32, retry_after: Option<u64>) -> Duration {
        let exponent = attempt.saturating_sub(1).min(20);
        let backoff = self.base.saturating_mul(1u32 << exponent).min(self.cap);
        backoff.max(Duration::from_secs(retry_after.unwrap_or(0)))
    }
}

/// The time source. Tests use a [`ManualClock`] to move time explicitly.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

pub struct ManualClock(Mutex<DateTime<Utc>>);
impl ManualClock {
    pub fn new(start: DateTime<Utc>) -> Self {
        Self(Mutex::new(start))
    }
    pub fn advance(&self, by: Duration) {
        let mut now = self.0.lock().unwrap_or_else(|e| e.into_inner());
        *now += chrono::Duration::from_std(by).expect("a reasonable duration");
    }
}
impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

struct Schedule {
    name: String,
    cron: cron::Schedule,
    make: Box<dyn Fn() -> ExecutionRequest + Send + Sync>,
    last_enqueued: Mutex<Option<DateTime<Utc>>>,
}

struct Inner {
    runtime: OnceLock<Arc<Runtime>>,
    store: Arc<dyn JobStore>,
    clock: Arc<dyn Clock>,
    retry: RetryPolicy,
    lease: Duration,
    poll_interval: Duration,
    schedules: Mutex<Vec<Arc<Schedule>>>,
    /// Unix seconds of the last worker tick, for readiness checks.
    last_tick: AtomicI64,
}

/// Handle to the queue. Cheap to clone; tools receive it as `State<Jobs>`.
#[derive(Clone)]
pub struct Jobs {
    inner: Arc<Inner>,
}

impl Jobs {
    pub fn new(runtime: Arc<Runtime>, store: Arc<dyn JobStore>) -> Self {
        let jobs = Self::unbound(store);
        let _ = jobs.inner.runtime.set(runtime);
        jobs
    }

    /// A queue whose runtime is bound later with [`Jobs::bind`]. Enqueueing works
    /// immediately; running jobs needs the runtime.
    pub fn unbound(store: Arc<dyn JobStore>) -> Self {
        Self {
            inner: Arc::new(Inner {
                runtime: OnceLock::new(),
                store,
                clock: Arc::new(SystemClock),
                retry: RetryPolicy::default(),
                lease: Duration::from_secs(60),
                poll_interval: Duration::from_millis(500),
                schedules: Mutex::new(Vec::new()),
                last_tick: AtomicI64::new(0),
            }),
        }
    }

    /// Bind the runtime that runs the jobs. Only the first bind takes effect.
    pub fn bind(&self, runtime: Arc<Runtime>) {
        let _ = self.inner.runtime.set(runtime);
    }

    fn runtime(&self) -> AgentResult<&Arc<Runtime>> {
        self.inner.runtime.get().ok_or_else(|| {
            AgentError::new(
                "JOBS_UNBOUND",
                "The job queue has no runtime; call Jobs::bind or Carmy::build first",
                ErrorCategory::Internal,
            )
        })
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        Arc::get_mut(&mut self.inner)
            .expect("configure before cloning")
            .clock = clock;
        self
    }

    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        Arc::get_mut(&mut self.inner)
            .expect("configure before cloning")
            .retry = retry;
        self
    }

    /// How long a claimed job stays owned by a worker without a heartbeat.
    pub fn with_lease(mut self, lease: Duration) -> Self {
        Arc::get_mut(&mut self.inner)
            .expect("configure before cloning")
            .lease = lease;
        self
    }

    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        Arc::get_mut(&mut self.inner)
            .expect("configure before cloning")
            .poll_interval = interval;
        self
    }

    pub fn now(&self) -> DateTime<Utc> {
        self.inner.clock.now()
    }

    /// Run `request` as soon as a worker is free.
    pub async fn enqueue(&self, request: ExecutionRequest) -> AgentResult<JobId> {
        let now = self.now();
        self.enqueue_at(request, now).await
    }

    pub async fn enqueue_after(
        &self,
        request: ExecutionRequest,
        delay: Duration,
    ) -> AgentResult<JobId> {
        let at = self.now() + chrono::Duration::from_std(delay).expect("a reasonable delay");
        self.enqueue_at(request, at).await
    }

    pub async fn enqueue_at(
        &self,
        request: ExecutionRequest,
        at: DateTime<Utc>,
    ) -> AgentResult<JobId> {
        self.enqueue_job(request, at, None).await
    }

    async fn enqueue_job(
        &self,
        request: ExecutionRequest,
        at: DateTime<Utc>,
        schedule: Option<String>,
    ) -> AgentResult<JobId> {
        let job = Job {
            id: JobId(format!("job_{}", uuid::Uuid::new_v4())),
            request: request.into(),
            run_at: at,
            created_at: self.now(),
            attempts: 0,
            max_attempts: self.inner.retry.max_attempts,
            status: JobStatus::Queued,
            last_error: None,
            lease_until: None,
            schedule,
        };
        self.inner.store.enqueue(job).await
    }

    /// Enqueue `make()` on a cron schedule (seven fields: sec min hour day month
    /// weekday year). Each occurrence gets `request_id = "{name}@{unix seconds}"`, so
    /// several workers enqueue it once. Occurrences missed by more than a minute are
    /// skipped.
    pub fn every(
        &self,
        name: impl Into<String>,
        expression: &str,
        make: impl Fn() -> ExecutionRequest + Send + Sync + 'static,
    ) -> AgentResult<()> {
        let cron = cron::Schedule::from_str(expression).map_err(|e| {
            AgentError::new(
                "INVALID_SCHEDULE",
                format!("`{expression}` is not a cron expression: {e}"),
                ErrorCategory::Validation,
            )
            .recoverable()
        })?;
        let schedule = Schedule {
            name: name.into(),
            cron,
            make: Box::new(make),
            last_enqueued: Mutex::new(None),
        };
        self.inner
            .schedules
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Arc::new(schedule));
        Ok(())
    }

    /// Cancel a job that has not started. Returns whether it was cancelled.
    pub async fn cancel(&self, id: &JobId) -> AgentResult<bool> {
        self.inner.store.cancel(id).await
    }

    pub async fn get(&self, id: &JobId) -> AgentResult<Option<Job>> {
        self.inner.store.get(id).await
    }

    pub async fn dead_letters(&self, limit: usize) -> AgentResult<Vec<Job>> {
        self.inner.store.dead_letters(limit).await
    }

    /// Whether a worker ticked within `within`.
    pub fn worker_alive(&self, within: Duration) -> bool {
        let last = self.inner.last_tick.load(Ordering::Relaxed);
        last > 0 && self.now().timestamp() - last <= within.as_secs() as i64
    }

    /// Enqueue every schedule occurrence that came due since the last tick.
    async fn tick_schedules(&self) -> AgentResult<()> {
        let now = self.now();
        let lookback = chrono::Duration::seconds(60);
        let schedules: Vec<Arc<Schedule>> = self
            .inner
            .schedules
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for schedule in schedules {
            let since = schedule
                .last_enqueued
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .unwrap_or(now - lookback)
                .max(now - lookback);
            let due: Vec<DateTime<Utc>> = schedule
                .cron
                .after(&since)
                .take_while(|at| *at <= now)
                .collect();
            for at in due {
                let mut request = (schedule.make)();
                request.request_id = Some(format!("{}@{}", schedule.name, at.timestamp()));
                self.enqueue_job(request, at, Some(schedule.name.clone()))
                    .await?;
                *schedule
                    .last_enqueued
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(at);
            }
        }
        Ok(())
    }

    /// One pass: enqueue due schedules, then claim and run up to `limit` due jobs to
    /// completion. Deterministic, for tests and single-shot workers. Returns how many
    /// jobs ran.
    pub async fn run_due(&self, limit: usize) -> AgentResult<usize> {
        self.runtime()?;
        self.tick_schedules().await?;
        self.inner
            .last_tick
            .store(self.now().timestamp(), Ordering::Relaxed);
        let jobs = self
            .inner
            .store
            .claim(self.now(), self.inner.lease, limit)
            .await?;
        let count = jobs.len();
        for job in jobs {
            self.run_job(job).await;
        }
        Ok(count)
    }

    /// The worker loop: schedules, claims and runs jobs with at most `concurrency` at
    /// once, until `shutdown` is cancelled. In-flight jobs finish before it returns.
    pub async fn work(&self, concurrency: usize, shutdown: CancellationToken) {
        if let Err(e) = self.runtime() {
            tracing::error!(error = %e, "the worker cannot start");
            return;
        }
        let permits = Arc::new(Semaphore::new(concurrency.max(1)));
        let mut running = tokio::task::JoinSet::new();
        loop {
            if let Err(e) = self.tick_schedules().await {
                tracing::warn!(error = %e, "schedules could not be enqueued");
            }
            self.inner
                .last_tick
                .store(self.now().timestamp(), Ordering::Relaxed);
            let free = permits.available_permits();
            let claimed = if free == 0 {
                Vec::new()
            } else {
                match self
                    .inner
                    .store
                    .claim(self.now(), self.inner.lease, free)
                    .await
                {
                    Ok(jobs) => jobs,
                    Err(e) => {
                        tracing::warn!(error = %e, "jobs could not be claimed");
                        Vec::new()
                    }
                }
            };
            let idle = claimed.is_empty();
            for job in claimed {
                let permit = permits.clone().acquire_owned().await.expect("never closed");
                let jobs = self.clone();
                running.spawn(async move {
                    let _permit = permit;
                    jobs.run_job(job).await;
                });
            }
            // Reap finished tasks so the set does not grow.
            while running.try_join_next().is_some() {}
            if idle {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = tokio::time::sleep(self.inner.poll_interval) => {}
                }
            } else if shutdown.is_cancelled() {
                break;
            }
        }
        // Graceful: in-flight jobs finish under their own deadlines. Cancelling them
        // would make their outcomes uncertain, which is worse than waiting.
        while running.join_next().await.is_some() {}
    }

    async fn run_job(&self, job: Job) {
        let Ok(runtime) = self.runtime() else {
            tracing::error!(job = %job.id, "no runtime bound; job left leased");
            return;
        };
        let span = tracing::info_span!(
            "carmy.job",
            job_id = %job.id,
            tool = %job.request.tool,
            attempt = job.attempts,
            schedule = job.schedule.as_deref(),
        );
        let _entered = span.enter();
        // Each attempt gets its own token: only the runtime deadline bounds it.
        let request = job
            .request
            .execution(job.attempts, CancellationToken::new());
        // Keep the lease alive while the tool runs.
        let heartbeat = {
            let (jobs, id, lease) = (self.clone(), job.id.clone(), self.inner.lease);
            tokio::spawn(async move {
                let interval = lease / 3;
                loop {
                    tokio::time::sleep(interval).await;
                    let until = jobs.now() + chrono::Duration::from_std(lease).expect("lease");
                    let _ = jobs.inner.store.heartbeat(&id, until).await;
                }
            })
        };
        let result = runtime.execute(request).await;
        heartbeat.abort();
        let outcome = self.outcome(&job, runtime, result.status, result.outcome.err());
        tracing::info!(outcome = ?outcome_name(&outcome), "job finished");
        if let Err(e) = self.inner.store.finish(&job.id, outcome).await {
            tracing::error!(job = %job.id, error = %e, "job outcome could not be recorded");
        }
    }

    /// The retry rule.
    fn outcome(
        &self,
        job: &Job,
        runtime: &Runtime,
        status: ExecutionStatus,
        error: Option<AgentError>,
    ) -> JobOutcome {
        let Some(error) = error else {
            return JobOutcome::Succeeded;
        };
        let exhausted = job.attempts >= job.max_attempts;
        let uncertain = matches!(
            status,
            ExecutionStatus::TimedOut | ExecutionStatus::Cancelled
        ) || error.code == "TOOL_PANIC";
        let retry_at = |retry_after| {
            let delay = self.inner.retry.delay(job.attempts, retry_after);
            self.now() + chrono::Duration::from_std(delay).expect("a bounded delay")
        };
        if uncertain {
            let idempotent = runtime
                .metadata(&job.request.tool)
                .is_some_and(|m| m.idempotent);
            let uncertain_error = AgentError::new(
                "EXECUTION_UNCERTAIN",
                "The attempt did not finish; external effects may have committed",
                ErrorCategory::Conflict,
            )
            .details(serde_json::json!({ "cause": error }));
            return if idempotent && !exhausted {
                JobOutcome::Retry {
                    at: retry_at(None),
                    error: uncertain_error,
                }
            } else {
                JobOutcome::DeadLettered(uncertain_error)
            };
        }
        if error.retryable {
            if exhausted {
                JobOutcome::DeadLettered(error)
            } else {
                JobOutcome::Retry {
                    at: retry_at(error.retry_after),
                    error,
                }
            }
        } else {
            JobOutcome::Failed(error)
        }
    }
}

fn outcome_name(outcome: &JobOutcome) -> &'static str {
    match outcome {
        JobOutcome::Succeeded => "succeeded",
        JobOutcome::Retry { .. } => "retry",
        JobOutcome::Failed(_) => "failed",
        JobOutcome::DeadLettered(_) => "dead_lettered",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backoff_grows_and_respects_retry_after() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.delay(1, None), Duration::from_secs(1));
        assert_eq!(policy.delay(2, None), Duration::from_secs(2));
        assert_eq!(policy.delay(4, None), Duration::from_secs(8));
        assert_eq!(policy.delay(2, Some(30)), Duration::from_secs(30));
        assert_eq!(policy.delay(40, None), Duration::from_secs(3600));
    }
    #[test]
    fn later_attempts_derive_their_request_id() {
        let request = carmy_runtime::execution_request("t", Value::Null);
        let mut job: JobRequest = request.into();
        job.request_id = Some("abc".into());
        let first = job.execution(1, CancellationToken::new());
        let third = job.execution(3, CancellationToken::new());
        assert_eq!(first.request_id.as_deref(), Some("abc"));
        assert_eq!(third.request_id.as_deref(), Some("abc#3"));
    }
}
