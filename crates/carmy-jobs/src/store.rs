//! Where jobs wait. Stores must make `enqueue` idempotent on `request_id`, and `claim`
//! atomic, so several workers never run the same job at once.
use crate::{Job, JobId, JobOutcome, JobStatus};
use carmy_core::{AgentError, AgentResult, ErrorCategory};
use chrono::{DateTime, Utc};
use std::{collections::HashMap, future::Future, pin::Pin, time::Duration};
use tokio::sync::Mutex;

pub type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = AgentResult<T>> + Send + 'a>>;

/// Durable or in-memory queue of jobs.
///
/// Contract:
/// - `enqueue` returns the id of an existing non-terminal job with the same
///   `request_id` instead of adding a duplicate;
/// - `claim` atomically marks up to `limit` due jobs `Running`, counts an attempt and
///   sets a lease; a `Running` job whose lease expired counts as due (its worker died);
/// - `finish` records the outcome; retries go back to `Queued` at the given time.
pub trait JobStore: Send + Sync {
    fn enqueue<'a>(&'a self, job: Job) -> StoreFuture<'a, JobId>;
    fn claim<'a>(
        &'a self,
        now: DateTime<Utc>,
        lease: Duration,
        limit: usize,
    ) -> StoreFuture<'a, Vec<Job>>;
    fn heartbeat<'a>(&'a self, id: &'a JobId, lease_until: DateTime<Utc>) -> StoreFuture<'a, ()>;
    fn finish<'a>(&'a self, id: &'a JobId, outcome: JobOutcome) -> StoreFuture<'a, ()>;
    fn cancel<'a>(&'a self, id: &'a JobId) -> StoreFuture<'a, bool>;
    fn get<'a>(&'a self, id: &'a JobId) -> StoreFuture<'a, Option<Job>>;
    fn dead_letters<'a>(&'a self, limit: usize) -> StoreFuture<'a, Vec<Job>>;
}

/// Process-local store for development and tests. Bounded; fails closed when full.
pub struct InMemoryJobStore {
    jobs: Mutex<HashMap<String, Job>>,
    capacity: usize,
}

impl InMemoryJobStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            jobs: Mutex::new(HashMap::new()),
            capacity,
        }
    }
}

impl Default for InMemoryJobStore {
    fn default() -> Self {
        Self::new(100_000)
    }
}

impl JobStore for InMemoryJobStore {
    fn enqueue<'a>(&'a self, job: Job) -> StoreFuture<'a, JobId> {
        Box::pin(async move {
            let mut jobs = self.jobs.lock().await;
            if let Some(request_id) = &job.request.request_id
                && let Some(existing) = jobs.values().find(|j| {
                    j.request.request_id.as_deref() == Some(request_id) && !j.status.is_terminal()
                })
            {
                return Ok(existing.id.clone());
            }
            if jobs.len() >= self.capacity {
                return Err(AgentError::new(
                    "JOBS_CAPACITY",
                    "Job store is full",
                    ErrorCategory::Capacity,
                )
                .retryable(Some(1)));
            }
            let id = job.id.clone();
            jobs.insert(id.0.clone(), job);
            Ok(id)
        })
    }

    fn claim<'a>(
        &'a self,
        now: DateTime<Utc>,
        lease: Duration,
        limit: usize,
    ) -> StoreFuture<'a, Vec<Job>> {
        Box::pin(async move {
            let mut jobs = self.jobs.lock().await;
            let mut due: Vec<&mut Job> = jobs
                .values_mut()
                .filter(|j| match j.status {
                    JobStatus::Queued => j.run_at <= now,
                    JobStatus::Running => j.lease_until.is_some_and(|until| until < now),
                    _ => false,
                })
                .collect();
            due.sort_by_key(|j| j.run_at);
            Ok(due
                .into_iter()
                .take(limit)
                .map(|job| {
                    job.status = JobStatus::Running;
                    job.attempts += 1;
                    job.lease_until = Some(now + lease);
                    job.clone()
                })
                .collect())
        })
    }

    fn heartbeat<'a>(&'a self, id: &'a JobId, lease_until: DateTime<Utc>) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            if let Some(job) = self.jobs.lock().await.get_mut(&id.0)
                && job.status == JobStatus::Running
            {
                job.lease_until = Some(lease_until);
            }
            Ok(())
        })
    }

    fn finish<'a>(&'a self, id: &'a JobId, outcome: JobOutcome) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let mut jobs = self.jobs.lock().await;
            let Some(job) = jobs.get_mut(&id.0) else {
                return Ok(());
            };
            job.lease_until = None;
            match outcome {
                JobOutcome::Succeeded => {
                    job.status = JobStatus::Succeeded;
                    job.last_error = None;
                }
                JobOutcome::Retry { at, error } => {
                    job.status = JobStatus::Queued;
                    job.run_at = at;
                    job.last_error = Some(error);
                }
                JobOutcome::Failed(error) => {
                    job.status = JobStatus::Failed;
                    job.last_error = Some(error);
                }
                JobOutcome::DeadLettered(error) => {
                    job.status = JobStatus::DeadLettered;
                    job.last_error = Some(error);
                }
            }
            Ok(())
        })
    }

    fn cancel<'a>(&'a self, id: &'a JobId) -> StoreFuture<'a, bool> {
        Box::pin(async move {
            let mut jobs = self.jobs.lock().await;
            match jobs.get_mut(&id.0) {
                Some(job) if job.status == JobStatus::Queued => {
                    job.status = JobStatus::Cancelled;
                    Ok(true)
                }
                _ => Ok(false),
            }
        })
    }

    fn get<'a>(&'a self, id: &'a JobId) -> StoreFuture<'a, Option<Job>> {
        Box::pin(async move { Ok(self.jobs.lock().await.get(&id.0).cloned()) })
    }

    fn dead_letters<'a>(&'a self, limit: usize) -> StoreFuture<'a, Vec<Job>> {
        Box::pin(async move {
            let jobs = self.jobs.lock().await;
            let mut dead: Vec<Job> = jobs
                .values()
                .filter(|j| j.status == JobStatus::DeadLettered)
                .cloned()
                .collect();
            dead.sort_by_key(|j| j.run_at);
            dead.truncate(limit);
            Ok(dead)
        })
    }
}
