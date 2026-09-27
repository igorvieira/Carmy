//! Contract tests against a real Redis. They need `CARMY_TEST_REDIS_URL` and skip
//! otherwise; CI provides one. Tests share the keys, so they run one at a time.
use carmy_core::*;
use carmy_jobs::{JobId, JobOutcome, JobStatus, JobStore, Jobs, ManualClock, RetryPolicy};
use carmy_redis::{Redis, RedisIdempotencyStore, RedisJobStore};
use carmy_runtime::{IdempotencyKey, IdempotencyStore, Reservation, Runtime, execution_request};
use chrono::{TimeZone, Utc};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// An emptied Redis, or `None` to skip.
async fn redis() -> Option<(Redis, String, tokio::sync::MutexGuard<'static, ()>)> {
    let Ok(url) = std::env::var("CARMY_TEST_REDIS_URL") else {
        eprintln!("CARMY_TEST_REDIS_URL is unset; skipping");
        return None;
    };
    let guard = LOCK.lock().await;
    let client = ::redis::Client::open(url.as_str()).unwrap();
    let mut connection = client.get_multiplexed_async_connection().await.unwrap();
    let _: () = ::redis::cmd("FLUSHDB")
        .query_async(&mut connection)
        .await
        .unwrap();
    Some((Redis::open(&url).unwrap(), url, guard))
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

async fn expire_lease(url: &str, id: &JobId) {
    let client = ::redis::Client::open(url).unwrap();
    let mut connection = client.get_multiplexed_async_connection().await.unwrap();
    let _: () = ::redis::cmd("ZADD")
        .arg("{carmy}:running")
        .arg(0)
        .arg(&id.0)
        .query_async(&mut connection)
        .await
        .unwrap();
}

#[tokio::test]
async fn jobs_enqueue_claim_and_finish() {
    let Some((redis, _, _guard)) = redis().await else {
        return;
    };
    let store = RedisJobStore::new(redis);
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let job = |input: &str, id: &str| {
        let mut j = Jobs::prepare(request(input).with_request_id(id), now, 3);
        j.created_at = now;
        j
    };
    let a = store.enqueue(job("a", "r-a")).await.unwrap();
    assert_eq!(
        a,
        store.enqueue(job("a", "r-a")).await.unwrap(),
        "idempotent enqueue"
    );
    let mut later = job("b", "r-b");
    later.run_at = now + chrono::Duration::seconds(60);
    let b = store.enqueue(later).await.unwrap();

    let claimed = store.claim(now, Duration::from_secs(30), 10).await.unwrap();
    assert_eq!(claimed.len(), 1, "only the due job");
    assert_eq!(claimed[0].id, a);
    assert_eq!(claimed[0].status, JobStatus::Running);
    assert_eq!(claimed[0].attempts, 1);
    assert!(claimed[0].lease_until.is_some());
    assert_eq!(claimed[0].request.arguments, json!("a"));
    assert!(
        store
            .claim(now, Duration::from_secs(30), 10)
            .await
            .unwrap()
            .is_empty()
    );

    store
        .finish(&a, JobOutcome::Succeeded(json!("ok")))
        .await
        .unwrap();
    let done = store.get(&a).await.unwrap().unwrap();
    assert_eq!(done.status, JobStatus::Succeeded);
    assert_eq!(done.result, Some(json!("ok")));
    assert!(done.lease_until.is_none());
    assert_ne!(
        store.enqueue(job("a", "r-a")).await.unwrap(),
        a,
        "a finished identity is free"
    );

    assert!(store.cancel(&b).await.unwrap());
    assert!(!store.cancel(&b).await.unwrap());
    assert_eq!(
        store.get(&b).await.unwrap().unwrap().status,
        JobStatus::Cancelled
    );
    assert!(store.get(&JobId("nope".into())).await.unwrap().is_none());
}

#[tokio::test]
async fn leases_follow_the_redis_clock() {
    let Some((redis, url, _guard)) = redis().await else {
        return;
    };
    let store = RedisJobStore::new(redis);
    let now = Utc::now();
    let id = store
        .enqueue(Jobs::prepare(request("a"), now, 3))
        .await
        .unwrap();
    store.claim(now, Duration::from_secs(30), 1).await.unwrap();
    let skewed = now + chrono::Duration::hours(1);
    assert!(
        store
            .claim(skewed, Duration::from_secs(30), 1)
            .await
            .unwrap()
            .is_empty(),
        "a worker an hour ahead still sees a live lease"
    );
    expire_lease(&url, &id).await;
    let second = store.claim(now, Duration::from_secs(30), 1).await.unwrap();
    assert_eq!(second[0].id, id);
    assert_eq!(second[0].attempts, 2);

    expire_lease(&url, &id).await;
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
async fn concurrent_claimers_never_share_a_job() {
    let Some((redis, _, _guard)) = redis().await else {
        return;
    };
    let store = Arc::new(RedisJobStore::new(redis));
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
    for claimer in claimers {
        for job in claimer.await.unwrap() {
            assert!(seen.insert(job.id), "a job was claimed twice");
        }
    }
    assert_eq!(seen.len(), 20);
}

#[tokio::test]
async fn a_queue_on_redis_retries_like_the_in_memory_one() {
    let Some((redis, _, _guard)) = redis().await else {
        return;
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let runtime = Arc::new(Runtime::new().tool(Counting(calls.clone())).unwrap());
    let clock = Arc::new(ManualClock::new(
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    ));
    let jobs = Jobs::new(runtime, Arc::new(RedisJobStore::new(redis)))
        .with_clock(clock.clone())
        .with_retry(RetryPolicy::default());
    let id = jobs
        .enqueue(request("busy").with_request_id("busy-1"))
        .await
        .unwrap();
    assert_eq!(jobs.run_due(10).await.unwrap(), 1);
    let job = jobs.get(&id).await.unwrap().unwrap();
    assert_eq!(job.status, JobStatus::Queued);
    assert_eq!(job.run_at, jobs.now() + chrono::Duration::seconds(5));
    clock.advance(Duration::from_secs(5));
    jobs.run_due(10).await.unwrap();
    clock.advance(Duration::from_secs(5));
    jobs.run_due(10).await.unwrap();
    let job = jobs.get(&id).await.unwrap().unwrap();
    assert_eq!(job.status, JobStatus::Succeeded);
    assert_eq!(job.attempts, 3);
    assert_eq!(job.result, Some(json!(3)));
}

#[tokio::test]
async fn purge_drops_old_finished_jobs_and_keeps_the_rest() {
    let Some((redis, _, _guard)) = redis().await else {
        return;
    };
    let store = RedisJobStore::new(redis);
    let now = Utc::now();
    let done = store
        .enqueue(Jobs::prepare(request("done"), now, 3))
        .await
        .unwrap();
    store.claim(now, Duration::from_secs(30), 1).await.unwrap();
    store
        .finish(&done, JobOutcome::Succeeded(json!(1)))
        .await
        .unwrap();
    let dead = store
        .enqueue(Jobs::prepare(request("dead"), now, 3))
        .await
        .unwrap();
    store.claim(now, Duration::from_secs(30), 1).await.unwrap();
    let error = AgentError::new("X", "x", ErrorCategory::Internal);
    store
        .finish(&dead, JobOutcome::DeadLettered(error))
        .await
        .unwrap();
    let queued = store
        .enqueue(Jobs::prepare(
            request("later"),
            now + chrono::Duration::days(1),
            3,
        ))
        .await
        .unwrap();

    assert_eq!(
        store.purge(now - chrono::Duration::hours(1)).await.unwrap(),
        0,
        "too recent"
    );
    assert_eq!(
        store
            .purge(Utc::now() + chrono::Duration::seconds(1))
            .await
            .unwrap(),
        1
    );
    assert!(store.get(&done).await.unwrap().is_none());
    assert!(
        store.get(&dead).await.unwrap().is_some(),
        "dead letters stay"
    );
    assert!(
        store.get(&queued).await.unwrap().is_some(),
        "queued jobs stay"
    );
}

#[tokio::test]
async fn idempotency_reserves_replays_expires_and_detects_conflicts() {
    let Some((redis, url, _guard)) = redis().await else {
        return;
    };
    let one = RedisIdempotencyStore::new(redis.clone());
    let two = RedisIdempotencyStore::new(redis).with_retention(Duration::from_secs(60));
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

    let client = ::redis::Client::open(url.as_str()).unwrap();
    let mut connection = client.get_multiplexed_async_connection().await.unwrap();
    let identity = serde_json::to_string(&(&key.principal, &key.session, &key.request_id)).unwrap();
    let stored = format!("{{carmy}}:idem:{identity}");
    let ttl: i64 = ::redis::cmd("PTTL")
        .arg(&stored)
        .query_async(&mut connection)
        .await
        .unwrap();
    assert_eq!(ttl, -1, "in-progress records never expire");

    let result = ExecutionResult {
        execution_id: "exec_1".into(),
        status: ExecutionStatus::Completed,
        outcome: Ok(json!({"order_id": 1})),
    };
    two.complete(&key, "fp", &result).await.unwrap();
    match one.reserve(&key, "fp").await.unwrap() {
        Reservation::Replay(replayed) => assert_eq!(replayed, result),
        other => panic!("expected a replay, got {other:?}"),
    }
    let ttl: i64 = ::redis::cmd("PTTL")
        .arg(&stored)
        .query_async(&mut connection)
        .await
        .unwrap();
    assert!(ttl > 0 && ttl <= 60_000, "completed records expire: {ttl}");
    assert_eq!(
        one.complete(&key, "fp", &result).await.unwrap_err().code,
        "IDEMPOTENCY_CONFLICT"
    );
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
async fn the_runtime_replays_through_redis() {
    let Some((redis, _, _guard)) = redis().await else {
        return;
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let runtime = Runtime::new()
        .idempotency_store(Arc::new(RedisIdempotencyStore::new(redis.clone())))
        .tool(Counting(calls.clone()))
        .unwrap();
    let req = request("x").with_request_id("rt-1");
    let first = runtime.execute(req.clone()).await;
    let second = runtime.execute(req).await;
    assert_eq!(first, second);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    carmy_redis::ready(redis).await.unwrap();
    let down = Redis::open("redis://127.0.0.1:1").unwrap();
    assert_eq!(
        carmy_redis::ready(down).await.unwrap_err().code,
        "DATABASE_UNAVAILABLE"
    );
}
