//! Contract tests against a real Postgres. They need `CARMY_TEST_DATABASE_URL` and skip
//! otherwise; CI provides a database. Tests share the tables, so they run one at a time.
use carmy_core::*;
use carmy_jobs::{JobOutcome, JobStatus, JobStore, Jobs, ManualClock, RetryPolicy};
use carmy_postgres::{
    Cleaned, PostgresAudit, PostgresIdempotencyStore, PostgresJobStore, Retention, cleanup,
    connect, migrate,
};
use carmy_runtime::{IdempotencyKey, IdempotencyStore, Reservation, Runtime, execution_request};
use chrono::{TimeZone, Utc};
use serde_json::json;
use sqlx::PgPool;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A migrated, emptied database, or `None` to skip.
async fn database() -> Option<(PgPool, tokio::sync::MutexGuard<'static, ()>)> {
    let Ok(url) = std::env::var("CARMY_TEST_DATABASE_URL") else {
        eprintln!("CARMY_TEST_DATABASE_URL is unset; skipping");
        return None;
    };
    let guard = LOCK.lock().await;
    let pool = connect(&url).await.expect("a reachable database");
    migrate(&pool).await.expect("migrations apply");
    sqlx::query("TRUNCATE carmy_jobs, carmy_idempotency, carmy_audit")
        .execute(&pool)
        .await
        .unwrap();
    Some((pool, guard))
}

struct Counting(Arc<AtomicUsize>);
impl Tool for Counting {
    type Input = String;
    type Output = usize;
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: "count".into(),
            description: "count".into(),
            input_schema: schemars::schema_for!(String).to_value(),
            output_schema: schemars::schema_for!(usize).to_value(),
            effect: Effect::Write,
            idempotent: false,
            parallel_safe: true,
            confirmation: Confirmation::None,
        }
    }
    async fn execute(&self, _: AgentContext, input: String) -> AgentResult<usize> {
        let n = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        if input == "busy" && n < 3 {
            return Err(
                AgentError::new("BUSY", "later", ErrorCategory::Capacity).retryable(Some(5))
            );
        }
        Ok(n)
    }
}

fn request(input: &str) -> ExecutionRequest {
    execution_request("count", json!(input))
}

#[tokio::test]
async fn jobs_enqueue_claim_and_finish() {
    let Some((pool, _guard)) = database().await else {
        return;
    };
    let store = PostgresJobStore::new(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let job = |input: &str, id: &str| {
        let mut j = Jobs::prepare(request(input).with_request_id(id), now, 3);
        j.created_at = now;
        j
    };
    let a = store.enqueue(job("a", "r-a")).await.unwrap();
    let again = store.enqueue(job("a", "r-a")).await.unwrap();
    assert_eq!(a, again, "enqueue is idempotent on request_id");
    let mut later = job("b", "r-b");
    later.run_at = now + chrono::Duration::seconds(60);
    let b = store.enqueue(later).await.unwrap();

    let claimed = store.claim(now, Duration::from_secs(30), 10).await.unwrap();
    assert_eq!(claimed.len(), 1, "only the due job is claimed");
    assert_eq!(claimed[0].id, a);
    assert_eq!(claimed[0].status, JobStatus::Running);
    assert_eq!(claimed[0].attempts, 1);
    assert_eq!(claimed[0].request.arguments, json!("a"));
    assert!(
        store
            .claim(now, Duration::from_secs(30), 10)
            .await
            .unwrap()
            .is_empty()
    );

    store.finish(&a, JobOutcome::Succeeded).await.unwrap();
    let done = store.get(&a).await.unwrap().unwrap();
    assert_eq!(done.status, JobStatus::Succeeded);
    assert!(done.lease_until.is_none());
    // A finished identity is free again.
    assert_ne!(store.enqueue(job("a", "r-a")).await.unwrap(), a);

    assert!(store.cancel(&b).await.unwrap());
    assert!(!store.cancel(&b).await.unwrap());
    assert_eq!(
        store.get(&b).await.unwrap().unwrap().status,
        JobStatus::Cancelled
    );
}

/// Simulates a worker whose lease ran out, on the database clock.
async fn expire_lease(pool: &PgPool, id: &carmy_jobs::JobId) {
    sqlx::query("UPDATE carmy_jobs SET lease_until = now() - interval '1 second' WHERE id = $1")
        .bind(&id.0)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn leases_follow_the_database_clock() {
    let Some((pool, _guard)) = database().await else {
        return;
    };
    let store = PostgresJobStore::new(pool.clone());
    let now = Utc::now();
    let id = store
        .enqueue(Jobs::prepare(request("a"), now, 3))
        .await
        .unwrap();
    let first = store.claim(now, Duration::from_secs(30), 1).await.unwrap();
    assert_eq!(first[0].attempts, 1);
    // A worker whose clock runs an hour ahead still sees a live lease.
    let skewed = now + chrono::Duration::hours(1);
    assert!(
        store
            .claim(skewed, Duration::from_secs(30), 1)
            .await
            .unwrap()
            .is_empty()
    );

    // The worker died: once the lease is over, another claim gets attempt 2.
    expire_lease(&pool, &id).await;
    let second = store.claim(now, Duration::from_secs(30), 1).await.unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].id, id);
    assert_eq!(second[0].attempts, 2);

    // Extending the lease keeps it owned, whatever the worker's clock says.
    expire_lease(&pool, &id).await;
    store
        .extend_lease(
            &id,
            now - chrono::Duration::hours(5),
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    assert!(
        store
            .claim(now, Duration::from_secs(30), 1)
            .await
            .unwrap()
            .is_empty()
    );

    let error = AgentError::new("BUSY", "gave up", ErrorCategory::Capacity);
    store
        .finish(&id, JobOutcome::DeadLettered(error))
        .await
        .unwrap();
    let dead = store.dead_letters(10).await.unwrap();
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].last_error.as_ref().unwrap().code, "BUSY");
}

#[tokio::test]
async fn migrations_are_carmys_own_and_safe_to_repeat() {
    let Some((pool, _guard)) = database().await else {
        return;
    };
    // Several instances starting together take turns.
    let runs = (0..4).map(|_| {
        let pool = pool.clone();
        tokio::spawn(async move { migrate(&pool).await })
    });
    for run in runs {
        run.await.unwrap().unwrap();
    }
    let versions: Vec<i32> =
        sqlx::query_scalar("SELECT version FROM carmy_schema_version ORDER BY version")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(versions, vec![1, 2, 3]);
    // The app's own sqlx migrations are none of Carmy's business.
    let touched: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
         WHERE table_name = '_sqlx_migrations') \
         AND EXISTS (SELECT 1 FROM _sqlx_migrations WHERE description LIKE 'carmy%' \
                     AND installed_on > now() - interval '1 minute')",
    )
    .fetch_one(&pool)
    .await
    .unwrap_or(false);
    assert!(!touched);
}

#[tokio::test]
async fn cleanup_deletes_only_what_retention_lets_go() {
    let Some((pool, _guard)) = database().await else {
        return;
    };
    let store = PostgresJobStore::new(pool.clone());
    let now = Utc::now();
    let mut ids = Vec::new();
    for (name, outcome) in [
        ("old-done", Some(JobOutcome::Succeeded)),
        ("new-done", Some(JobOutcome::Succeeded)),
        (
            "old-dead",
            Some(JobOutcome::DeadLettered(AgentError::new(
                "X",
                "x",
                ErrorCategory::Internal,
            ))),
        ),
        ("old-queued", None),
    ] {
        let id = store
            .enqueue(Jobs::prepare(request(name).with_request_id(name), now, 3))
            .await
            .unwrap();
        if let Some(outcome) = outcome {
            store.claim(now, Duration::from_secs(30), 10).await.unwrap();
            store.finish(&id, outcome).await.unwrap();
        }
        ids.push(id);
    }
    let age = |id: &carmy_jobs::JobId| {
        let pool = pool.clone();
        let id = id.0.clone();
        async move {
            sqlx::query(
                "UPDATE carmy_jobs SET finished_at = finished_at - interval '40 days', \
                 run_at = run_at - interval '40 days' WHERE id = $1",
            )
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        }
    };
    age(&ids[0]).await;
    age(&ids[2]).await;
    age(&ids[3]).await;

    let idempotency = PostgresIdempotencyStore::new(pool.clone());
    let key = |id: &str| IdempotencyKey {
        principal: None,
        session: None,
        request_id: id.into(),
    };
    let result = ExecutionResult {
        execution_id: "exec".into(),
        status: ExecutionStatus::Completed,
        outcome: Ok(json!(1)),
    };
    for id in ["old-complete", "old-running", "new-complete"] {
        idempotency.reserve(&key(id), "fp").await.unwrap();
        if id != "old-running" {
            idempotency.complete(&key(id), "fp", &result).await.unwrap();
        }
    }
    sqlx::query(
        "UPDATE carmy_idempotency SET created_at = now() - interval '10 days' \
         WHERE key LIKE '%old-%'",
    )
    .execute(&pool)
    .await
    .unwrap();

    let audit = Arc::new(PostgresAudit::new(pool.clone()));
    let runtime = Runtime::new()
        .sink(audit.clone())
        .tool(Counting(Arc::new(AtomicUsize::new(0))))
        .unwrap();
    runtime.execute(request("a")).await;
    runtime.execute(request("b")).await;
    for _ in 0..50 {
        if audit.recent(10).await.unwrap().len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    sqlx::query(
        "UPDATE carmy_audit SET started_at = now() - interval '100 days' \
         WHERE ctid IN (SELECT ctid FROM carmy_audit LIMIT 1)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let cleaned = cleanup(&pool, Retention::default()).await.unwrap();
    assert_eq!(
        cleaned,
        Cleaned {
            jobs: 1,
            idempotency: 1,
            audit: 1
        }
    );
    assert!(
        store.get(&ids[0]).await.unwrap().is_none(),
        "old finished job"
    );
    assert!(
        store.get(&ids[1]).await.unwrap().is_some(),
        "recent finished job"
    );
    assert!(
        store.get(&ids[2]).await.unwrap().is_some(),
        "dead letters stay"
    );
    assert!(
        store.get(&ids[3]).await.unwrap().is_some(),
        "queued jobs stay"
    );
    assert!(
        matches!(
            idempotency
                .reserve(&key("old-running"), "fp")
                .await
                .unwrap(),
            Reservation::InProgress
        ),
        "in-progress records stay"
    );
    assert!(matches!(
        idempotency
            .reserve(&key("new-complete"), "fp")
            .await
            .unwrap(),
        Reservation::Replay(_)
    ));
    assert!(matches!(
        idempotency
            .reserve(&key("old-complete"), "fp")
            .await
            .unwrap(),
        Reservation::Acquired
    ));
    assert_eq!(audit.recent(10).await.unwrap().len(), 1);
    assert_eq!(
        cleanup(&pool, Retention::default()).await.unwrap(),
        Cleaned::default()
    );
}

#[tokio::test]
async fn concurrent_claimers_never_share_a_job() {
    let Some((pool, _guard)) = database().await else {
        return;
    };
    let store = Arc::new(PostgresJobStore::new(pool.clone()));
    let now = Utc::now();
    for i in 0..20 {
        store
            .enqueue(Jobs::prepare(request(&i.to_string()), now, 3))
            .await
            .unwrap();
    }
    let claimers = (0..4).map(|_| {
        let store = store.clone();
        tokio::spawn(async move { store.claim(now, Duration::from_secs(30), 10).await.unwrap() })
    });
    let mut seen = std::collections::HashSet::new();
    let mut total = 0;
    for claimer in claimers {
        for job in claimer.await.unwrap() {
            assert!(seen.insert(job.id), "a job was claimed twice");
            total += 1;
        }
    }
    assert_eq!(total, 20);
}

#[tokio::test]
async fn the_outbox_follows_the_transaction() {
    let Some((pool, _guard)) = database().await else {
        return;
    };
    let store = PostgresJobStore::new(pool.clone());
    let now = Utc::now();
    // Rolled back: no job.
    let mut tx = pool.begin().await.unwrap();
    let rolled = store
        .enqueue_in(&mut tx, request("x").with_request_id("outbox-1"), now, 3)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert!(store.get(&rolled).await.unwrap().is_none());
    // Committed: the job exists exactly once.
    let mut tx = pool.begin().await.unwrap();
    let committed = store
        .enqueue_in(&mut tx, request("x").with_request_id("outbox-1"), now, 3)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        store.get(&committed).await.unwrap().unwrap().status,
        JobStatus::Queued
    );
}

#[tokio::test]
async fn a_queue_on_postgres_retries_and_replays_like_the_in_memory_one() {
    let Some((pool, _guard)) = database().await else {
        return;
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let runtime = Arc::new(Runtime::new().tool(Counting(calls.clone())).unwrap());
    let clock = Arc::new(ManualClock::new(
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    ));
    let jobs = Jobs::new(runtime, Arc::new(PostgresJobStore::new(pool.clone())))
        .with_clock(clock.clone())
        .with_retry(RetryPolicy::default());
    let id = jobs
        .enqueue(request("busy").with_request_id("busy-1"))
        .await
        .unwrap();
    assert_eq!(jobs.run_due(10).await.unwrap(), 1);
    let job = jobs.get(&id).await.unwrap().unwrap();
    assert_eq!(job.status, JobStatus::Queued);
    assert_eq!(job.attempts, 1);
    assert_eq!(job.run_at, jobs.now() + chrono::Duration::seconds(5));
    clock.advance(Duration::from_secs(5));
    assert_eq!(jobs.run_due(10).await.unwrap(), 1);
    clock.advance(Duration::from_secs(5));
    assert_eq!(jobs.run_due(10).await.unwrap(), 1);
    let job = jobs.get(&id).await.unwrap().unwrap();
    assert_eq!(job.status, JobStatus::Succeeded);
    assert_eq!(job.attempts, 3);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn idempotency_store_reserves_replays_and_detects_conflicts() {
    let Some((pool, _guard)) = database().await else {
        return;
    };
    // Two instances of the same service share the rows.
    let one = PostgresIdempotencyStore::new(pool.clone());
    let two = PostgresIdempotencyStore::new(pool.clone());
    let key = IdempotencyKey {
        principal: Some("ada".into()),
        session: None,
        request_id: "req-1".into(),
    };
    assert!(matches!(
        one.reserve(&key, "fp").await.unwrap(),
        Reservation::Acquired
    ));
    assert!(matches!(
        two.reserve(&key, "fp").await.unwrap(),
        Reservation::InProgress
    ));
    assert!(matches!(
        two.reserve(&key, "other").await.unwrap(),
        Reservation::Conflict
    ));
    let result = ExecutionResult {
        execution_id: "exec_1".into(),
        status: ExecutionStatus::Completed,
        outcome: Ok(json!({"order_id": 1})),
    };
    one.complete(&key, "fp", &result).await.unwrap();
    match two.reserve(&key, "fp").await.unwrap() {
        Reservation::Replay(replayed) => assert_eq!(replayed, result),
        other => panic!("expected a replay, got {other:?}"),
    }
    // Completing twice, or with another fingerprint, is a conflict.
    assert_eq!(
        one.complete(&key, "fp", &result).await.unwrap_err().code,
        "IDEMPOTENCY_CONFLICT"
    );
    // A different session is a different identity.
    let other_session = IdempotencyKey {
        session: Some("s2".into()),
        ..key.clone()
    };
    assert!(matches!(
        one.reserve(&other_session, "fp").await.unwrap(),
        Reservation::Acquired
    ));
}

#[tokio::test]
async fn the_runtime_replays_through_postgres() {
    let Some((pool, _guard)) = database().await else {
        return;
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let runtime = Runtime::new()
        .idempotency_store(Arc::new(PostgresIdempotencyStore::new(pool)))
        .tool(Counting(calls.clone()))
        .unwrap();
    let req = request("x").with_request_id("rt-1");
    let first = runtime.execute(req.clone()).await;
    let second = runtime.execute(req).await;
    assert_eq!(first, second);
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the tool ran once");
}

#[tokio::test]
async fn the_audit_trail_is_written_off_the_execution_path() {
    let Some((pool, _guard)) = database().await else {
        return;
    };
    let audit = Arc::new(PostgresAudit::new(pool.clone()));
    let calls = Arc::new(AtomicUsize::new(0));
    let runtime = Runtime::new()
        .sink(audit.clone())
        .tool(Counting(calls.clone()))
        .unwrap();
    let mut req = request("x").with_request_id("audit-1");
    req.context.principal = Some("ada".into());
    runtime.execute(req.clone()).await;
    runtime.execute(req).await;
    runtime.execute(execution_request("count", json!(42))).await; // invalid arguments

    // The writer is asynchronous: wait for the rows without blocking the executions.
    let mut records = Vec::new();
    for _ in 0..50 {
        records = audit.recent(10).await.unwrap();
        if records.len() == 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].error_code.as_deref(), Some("INVALID_ARGUMENTS"));
    assert_eq!(records[0].status, ExecutionStatus::Failed);
    assert!(records[1].replayed);
    assert_eq!(records[1].execution_id, records[2].execution_id);
    assert_eq!(records[2].principal.as_deref(), Some("ada"));
    assert_eq!(records[2].effect, Some(Effect::Write));
    assert_eq!(records[2].request_id.as_deref(), Some("audit-1"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
