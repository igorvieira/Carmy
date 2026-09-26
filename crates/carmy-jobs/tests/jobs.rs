//! Contract tests for the queue, with a manual clock so time moves explicitly.
use carmy_core::*;
use carmy_jobs::{InMemoryJobStore, JobStatus, JobStore, Jobs, ManualClock, RetryPolicy};
use carmy_runtime::{Runtime, execution_request};
use chrono::{TimeZone, Utc};
use serde_json::json;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

/// A tool whose behaviour is chosen by name; every call is recorded.
struct Scripted {
    name: &'static str,
    idempotent: bool,
    calls: Arc<AtomicUsize>,
    log: Arc<Mutex<Vec<String>>>,
    /// How many calls fail with a retryable error before succeeding.
    fail_times: usize,
    delay: Duration,
    panics: bool,
    fatal: bool,
}
impl Tool for Scripted {
    type Input = String;
    type Output = String;
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: self.name.into(),
            description: self.name.into(),
            input_schema: schemars::schema_for!(String).to_value(),
            output_schema: schemars::schema_for!(String).to_value(),
            effect: Effect::Write,
            idempotent: self.idempotent,
            parallel_safe: true,
            confirmation: Confirmation::None,
        }
    }
    async fn execute(&self, _: AgentContext, input: String) -> AgentResult<String> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        self.log.lock().unwrap().push(input.clone());
        if self.panics {
            panic!("tool bug");
        }
        if self.fatal {
            return Err(AgentError::new(
                "BAD",
                "permanent",
                ErrorCategory::Validation,
            ));
        }
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        if call <= self.fail_times {
            return Err(
                AgentError::new("BUSY", "try later", ErrorCategory::Capacity).retryable(Some(30)),
            );
        }
        Ok(input)
    }
}

struct Harness {
    jobs: Jobs,
    store: Arc<InMemoryJobStore>,
    clock: Arc<ManualClock>,
    calls: Arc<AtomicUsize>,
    log: Arc<Mutex<Vec<String>>>,
}

fn harness(configure: impl FnOnce(Scripted) -> Scripted, retry: RetryPolicy) -> Harness {
    let calls = Arc::new(AtomicUsize::new(0));
    let log = Arc::new(Mutex::new(Vec::new()));
    let base = |name: &'static str| Scripted {
        name,
        idempotent: false,
        calls: calls.clone(),
        log: log.clone(),
        fail_times: 0,
        delay: Duration::ZERO,
        panics: false,
        fatal: false,
    };
    let runtime = Arc::new(
        Runtime::new()
            .timeout(Duration::from_millis(50))
            .tool(configure(base("work")))
            .unwrap()
            .tool(Scripted {
                idempotent: true,
                ..base("stable")
            })
            .unwrap(),
    );
    let store = Arc::new(InMemoryJobStore::default());
    let clock = Arc::new(ManualClock::new(
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    ));
    let jobs = Jobs::new(runtime, store.clone())
        .with_clock(clock.clone())
        .with_retry(retry)
        .with_lease(Duration::from_secs(10));
    Harness {
        jobs,
        store,
        clock,
        calls,
        log,
    }
}

fn request(tool: &str, input: &str) -> ExecutionRequest {
    execution_request(tool, json!(input))
}

#[tokio::test]
async fn runs_due_jobs_in_order() {
    let h = harness(|t| t, RetryPolicy::default());
    let later = h
        .jobs
        .enqueue_after(request("work", "later"), Duration::from_secs(10))
        .await
        .unwrap();
    let now = h.jobs.enqueue(request("work", "now")).await.unwrap();
    assert_eq!(h.jobs.run_due(10).await.unwrap(), 1);
    assert_eq!(
        h.jobs.get(&now).await.unwrap().unwrap().status,
        JobStatus::Succeeded
    );
    assert_eq!(
        h.jobs.get(&later).await.unwrap().unwrap().status,
        JobStatus::Queued
    );
    h.clock.advance(Duration::from_secs(10));
    assert_eq!(h.jobs.run_due(10).await.unwrap(), 1);
    assert_eq!(*h.log.lock().unwrap(), vec!["now", "later"]);
}

#[tokio::test]
async fn enqueue_is_idempotent_on_request_id() {
    let h = harness(|t| t, RetryPolicy::default());
    let req = || request("work", "x").with_request_id("order-1");
    let a = h.jobs.enqueue(req()).await.unwrap();
    let b = h.jobs.enqueue(req()).await.unwrap();
    assert_eq!(a, b, "a queued job with the same request_id is reused");
    h.jobs.run_due(10).await.unwrap();
    assert_eq!(h.calls.load(Ordering::SeqCst), 1);
    // Once finished, the identity is free again (the runtime replays it anyway).
    let c = h.jobs.enqueue(req()).await.unwrap();
    assert_ne!(a, c);
}

#[tokio::test]
async fn retries_with_backoff_and_retry_after() {
    let h = harness(|t| Scripted { fail_times: 2, ..t }, RetryPolicy::default());
    let id = h
        .jobs
        .enqueue(request("work", "x").with_request_id("r"))
        .await
        .unwrap();
    h.jobs.run_due(10).await.unwrap();
    let job = h.jobs.get(&id).await.unwrap().unwrap();
    assert_eq!(job.status, JobStatus::Queued);
    assert_eq!(job.attempts, 1);
    assert_eq!(job.last_error.as_ref().unwrap().code, "BUSY");
    // retry_after (30 s) beats the 1 s backoff.
    assert_eq!(job.run_at, h.jobs.now() + chrono::Duration::seconds(30));
    h.clock.advance(Duration::from_secs(29));
    assert_eq!(h.jobs.run_due(10).await.unwrap(), 0);
    h.clock.advance(Duration::from_secs(1));
    assert_eq!(h.jobs.run_due(10).await.unwrap(), 1);
    h.clock.advance(Duration::from_secs(30));
    assert_eq!(h.jobs.run_due(10).await.unwrap(), 1);
    let job = h.jobs.get(&id).await.unwrap().unwrap();
    assert_eq!(job.status, JobStatus::Succeeded);
    assert_eq!(job.attempts, 3);
    assert!(job.last_error.is_none());
    assert_eq!(h.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn dead_letters_after_max_attempts() {
    let policy = RetryPolicy {
        max_attempts: 2,
        ..RetryPolicy::default()
    };
    let h = harness(
        |t| Scripted {
            fail_times: 99,
            ..t
        },
        policy,
    );
    let id = h.jobs.enqueue(request("work", "x")).await.unwrap();
    h.jobs.run_due(10).await.unwrap();
    h.clock.advance(Duration::from_secs(60));
    h.jobs.run_due(10).await.unwrap();
    let job = h.jobs.get(&id).await.unwrap().unwrap();
    assert_eq!(job.status, JobStatus::DeadLettered);
    assert_eq!(job.attempts, 2);
    let dead = h.jobs.dead_letters(10).await.unwrap();
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].last_error.as_ref().unwrap().code, "BUSY");
    h.clock.advance(Duration::from_secs(3600));
    assert_eq!(
        h.jobs.run_due(10).await.unwrap(),
        0,
        "dead letters never run again"
    );
}

#[tokio::test]
async fn non_retryable_errors_fail_immediately() {
    let h = harness(|t| Scripted { fatal: true, ..t }, RetryPolicy::default());
    let id = h.jobs.enqueue(request("work", "x")).await.unwrap();
    h.jobs.run_due(10).await.unwrap();
    let job = h.jobs.get(&id).await.unwrap().unwrap();
    assert_eq!(job.status, JobStatus::Failed);
    assert_eq!(job.attempts, 1);
    assert_eq!(job.last_error.unwrap().code, "BAD");
    assert!(h.jobs.dead_letters(10).await.unwrap().is_empty());
}

#[tokio::test]
async fn uncertain_outcomes_dead_letter_unless_the_tool_is_idempotent() {
    // The runtime times out after 50 ms; `work` is not idempotent, `stable` is.
    let h = harness(
        |t| Scripted {
            delay: Duration::from_secs(5),
            ..t
        },
        RetryPolicy::default(),
    );
    let risky = h.jobs.enqueue(request("work", "x")).await.unwrap();
    h.jobs.run_due(10).await.unwrap();
    let job = h.jobs.get(&risky).await.unwrap().unwrap();
    assert_eq!(job.status, JobStatus::DeadLettered);
    let error = job.last_error.unwrap();
    assert_eq!(error.code, "EXECUTION_UNCERTAIN");
    assert_eq!(error.details.unwrap()["cause"]["code"], "TIMEOUT");

    let h = harness(|t| t, RetryPolicy::default());
    let stable = h.jobs.enqueue(request("stable", "x")).await.unwrap();
    // Cancel through the runtime deadline again, this time on an idempotent tool.
    let store = h.store.clone();
    let claimed = store
        .claim(h.jobs.now(), Duration::from_secs(10), 1)
        .await
        .unwrap();
    assert_eq!(claimed[0].id, stable);
    // Simulate a worker that died mid-flight: the lease expires and the job is retried.
    h.clock.advance(Duration::from_secs(11));
    assert_eq!(h.jobs.run_due(10).await.unwrap(), 1);
    let job = h.jobs.get(&stable).await.unwrap().unwrap();
    assert_eq!(job.status, JobStatus::Succeeded);
    assert_eq!(job.attempts, 2, "the dead worker's attempt counts");
}

#[tokio::test]
async fn panics_are_uncertain() {
    let h = harness(|t| Scripted { panics: true, ..t }, RetryPolicy::default());
    let id = h.jobs.enqueue(request("work", "x")).await.unwrap();
    h.jobs.run_due(10).await.unwrap();
    let job = h.jobs.get(&id).await.unwrap().unwrap();
    assert_eq!(job.status, JobStatus::DeadLettered);
    assert_eq!(
        job.last_error.unwrap().details.unwrap()["cause"]["code"],
        "TOOL_PANIC"
    );
}

#[tokio::test]
async fn schedules_enqueue_each_occurrence_once_across_workers() {
    let h = harness(|t| t, RetryPolicy::default());
    let other = h.jobs.clone();
    for jobs in [&h.jobs, &other] {
        jobs.every("tick", "0 * * * * * *", || request("work", "tick"))
            .unwrap();
    }
    // Both workers tick at 00:00:00: one occurrence is due, and it runs once.
    let (a, b) = tokio::join!(h.jobs.run_due(10), other.run_due(10));
    assert_eq!(a.unwrap() + b.unwrap(), 1);
    assert_eq!(h.calls.load(Ordering::SeqCst), 1);
    h.clock.advance(Duration::from_secs(60));
    let (a, b) = tokio::join!(h.jobs.run_due(10), other.run_due(10));
    assert_eq!(a.unwrap() + b.unwrap(), 1);
    assert_eq!(h.calls.load(Ordering::SeqCst), 2);
    // A bad expression is rejected up front.
    assert_eq!(
        h.jobs
            .every("bad", "every day", || request("work", "x"))
            .unwrap_err()
            .code,
        "INVALID_SCHEDULE"
    );
}

#[tokio::test]
async fn queued_jobs_can_be_cancelled() {
    let h = harness(|t| t, RetryPolicy::default());
    let id = h
        .jobs
        .enqueue_after(request("work", "x"), Duration::from_secs(3600))
        .await
        .unwrap();
    assert!(h.jobs.cancel(&id).await.unwrap());
    assert!(!h.jobs.cancel(&id).await.unwrap());
    assert_eq!(
        h.jobs.get(&id).await.unwrap().unwrap().status,
        JobStatus::Cancelled
    );
    h.clock.advance(Duration::from_secs(3600));
    assert_eq!(h.jobs.run_due(10).await.unwrap(), 0);
    assert_eq!(h.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_worker_loop_finishes_in_flight_jobs_on_shutdown() {
    let calls = Arc::new(AtomicUsize::new(0));
    let runtime = Arc::new(
        Runtime::new()
            .tool(Scripted {
                name: "slow",
                idempotent: true,
                calls: calls.clone(),
                log: Arc::default(),
                fail_times: 0,
                delay: Duration::from_millis(200),
                panics: false,
                fatal: false,
            })
            .unwrap(),
    );
    let jobs = Jobs::new(runtime, Arc::new(InMemoryJobStore::default()))
        .with_poll_interval(Duration::from_millis(10));
    let id = jobs.enqueue(request("slow", "x")).await.unwrap();
    let shutdown = CancellationToken::new();
    let worker = tokio::spawn({
        let (jobs, shutdown) = (jobs.clone(), shutdown.clone());
        async move { jobs.work(2, shutdown).await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        jobs.get(&id).await.unwrap().unwrap().status,
        JobStatus::Running
    );
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(2), worker)
        .await
        .expect("the worker returns after shutdown")
        .unwrap();
    assert_eq!(
        jobs.get(&id).await.unwrap().unwrap().status,
        JobStatus::Succeeded
    );
    assert!(jobs.worker_alive(Duration::from_secs(5)));
}

#[tokio::test]
async fn an_unbound_queue_enqueues_but_cannot_run() {
    let jobs = Jobs::unbound(Arc::new(InMemoryJobStore::default()));
    jobs.enqueue(request("work", "x")).await.unwrap();
    assert_eq!(jobs.run_due(10).await.unwrap_err().code, "JOBS_UNBOUND");
}
