//! Durable stores on Redis: jobs and idempotency, shared by every instance of a service.
//!
//! ```no_run
//! # fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let redis = carmy_redis::Redis::open("redis://localhost")?;   // connects on first use
//! let jobs = std::sync::Arc::new(carmy_redis::RedisJobStore::new(redis.clone()));
//! let idempotency = std::sync::Arc::new(carmy_redis::RedisIdempotencyStore::new(redis));
//! # Ok(()) }
//! ```
//!
//! Every multi-step change runs as one Lua script, so it is atomic. Leases use the
//! Redis clock (`TIME`), as the Postgres store uses the database's; due times follow
//! the app's clock. Keys share the `{carmy}` hash tag, so a cluster keeps them in one
//! slot. Completed idempotency records expire after a retention (seven days by
//! default); in-progress records never expire. There is no Redis audit sink: the audit
//! trail stays in memory, or goes to Postgres.
use carmy_core::{AgentError, AgentResult, ErrorCategory, ExecutionResult};
use carmy_jobs::{Job, JobId, JobOutcome, JobStatus, JobStore, StoreFuture};
use carmy_runtime::{IdempotencyKey, IdempotencyStore, Reservation};
use chrono::{DateTime, TimeZone, Utc};
use redis::{AsyncCommands, Script, aio::ConnectionManager};
use std::{sync::Arc, time::Duration};

/// Key prefix. The braces are a hash tag: in a cluster, every Carmy key hashes to the
/// same slot, which the scripts need.
const PREFIX: &str = "{carmy}";

/// A Redis connection that opens on first use and reconnects on its own. Cheap to
/// clone; clones share the connection.
#[derive(Clone)]
pub struct Redis {
    client: redis::Client,
    connection: Arc<tokio::sync::OnceCell<ConnectionManager>>,
}

impl Redis {
    /// Parse `url` (`redis://`, `rediss://` for TLS). Nothing connects yet.
    pub fn open(url: &str) -> Result<Self, redis::RedisError> {
        Ok(Self {
            client: redis::Client::open(url)?,
            connection: Arc::new(tokio::sync::OnceCell::new()),
        })
    }

    async fn connection(&self) -> AgentResult<ConnectionManager> {
        self.connection
            .get_or_try_init(|| async {
                // Fail fast when Redis is down: readiness and callers get an answer
                // in seconds, and the manager keeps reconnecting in the background.
                let config = redis::aio::ConnectionManagerConfig::new()
                    .set_connection_timeout(Some(Duration::from_secs(2)))
                    .set_response_timeout(Some(Duration::from_secs(5)))
                    .set_number_of_retries(2)
                    .set_max_delay(Duration::from_millis(500));
                ConnectionManager::new_with_config(self.client.clone(), config)
                    .await
                    .map_err(store_error)
            })
            .await
            .cloned()
    }
}

/// A readiness check: `PING` must answer.
pub async fn ready(redis: Redis) -> AgentResult<()> {
    let mut connection = redis.connection().await.map_err(|e| {
        AgentError::new(
            "DATABASE_UNAVAILABLE",
            format!("Redis did not answer: {}", e.message),
            ErrorCategory::Capacity,
        )
        .retryable(Some(1))
    })?;
    let _: String = redis::cmd("PING")
        .query_async(&mut connection)
        .await
        .map_err(|e| {
            AgentError::new(
                "DATABASE_UNAVAILABLE",
                format!("Redis did not answer: {e}"),
                ErrorCategory::Capacity,
            )
            .retryable(Some(1))
        })?;
    Ok(())
}

fn store_error(e: impl std::fmt::Display) -> AgentError {
    AgentError::new(
        "STORE_ERROR",
        format!("Redis store failed: {e}"),
        ErrorCategory::Internal,
    )
    .retryable(Some(1))
}

fn millis(at: DateTime<Utc>) -> i64 {
    at.timestamp_millis()
}

fn key(kind: &str, id: &str) -> String {
    format!("{PREFIX}:{kind}:{id}")
}

// ---------------------------------------------------------------- jobs

const QUEUE: &str = "{carmy}:queue";
const RUNNING: &str = "{carmy}:running";
const DEAD: &str = "{carmy}:dead";
const FINISHED: &str = "{carmy}:finished";

/// Milliseconds on the Redis clock, in Lua.
const NOW: &str = "local t = redis.call('TIME')
local now = tonumber(t[1]) * 1000 + math.floor(tonumber(t[2]) / 1000)
";

/// [`JobStore`] on Redis. A job is a hash (`json`, `status`, `attempts`); sorted sets
/// index queued jobs by `run_at`, running ones by lease end, and finished and
/// dead-lettered ones by time.
#[derive(Clone)]
pub struct RedisJobStore {
    redis: Redis,
}

impl RedisJobStore {
    pub fn new(redis: Redis) -> Self {
        Self { redis }
    }
}

fn parse_status(status: &str) -> AgentResult<JobStatus> {
    Ok(match status {
        "queued" => JobStatus::Queued,
        "running" => JobStatus::Running,
        "succeeded" => JobStatus::Succeeded,
        "failed" => JobStatus::Failed,
        "dead_lettered" => JobStatus::DeadLettered,
        "cancelled" => JobStatus::Cancelled,
        other => {
            return Err(AgentError::new(
                "STORE_ERROR",
                format!("unknown job status `{other}`"),
                ErrorCategory::Internal,
            ));
        }
    })
}

impl RedisJobStore {
    async fn load(&self, connection: &mut ConnectionManager, id: &str) -> AgentResult<Option<Job>> {
        let fields: std::collections::HashMap<String, String> = connection
            .hgetall(key("job", id))
            .await
            .map_err(store_error)?;
        let Some(json) = fields.get("json") else {
            return Ok(None);
        };
        let mut job: Job = serde_json::from_str(json).map_err(store_error)?;
        if let Some(status) = fields.get("status") {
            job.status = parse_status(status)?;
        }
        if let Some(attempts) = fields.get("attempts") {
            job.attempts = attempts.parse().map_err(store_error)?;
        }
        job.lease_until = None;
        if job.status == JobStatus::Running {
            let lease: Option<i64> = connection.zscore(RUNNING, id).await.map_err(store_error)?;
            job.lease_until = lease.and_then(|ms| Utc.timestamp_millis_opt(ms).single());
        }
        Ok(Some(job))
    }
}

impl JobStore for RedisJobStore {
    fn enqueue<'a>(&'a self, job: Job) -> StoreFuture<'a, JobId> {
        Box::pin(async move {
            let mut connection = self.redis.connection().await?;
            let script = Script::new(
                "if ARGV[5] == '1' then
                   local existing = redis.call('GET', KEYS[3])
                   if existing then return existing end
                 end
                 redis.call('HSET', KEYS[1], 'json', ARGV[2], 'status', 'queued', 'attempts', ARGV[4])
                 redis.call('ZADD', KEYS[2], ARGV[3], ARGV[1])
                 if ARGV[5] == '1' then redis.call('SET', KEYS[3], ARGV[1]) end
                 return ARGV[1]",
            );
            let json = serde_json::to_string(&job).map_err(store_error)?;
            let request_id = job.request.request_id.clone();
            let id: String = script
                .key(key("job", &job.id.0))
                .key(QUEUE)
                .key(key("active", request_id.as_deref().unwrap_or("")))
                .arg(&job.id.0)
                .arg(json)
                .arg(millis(job.run_at))
                .arg(job.attempts)
                .arg(if request_id.is_some() { "1" } else { "0" })
                .invoke_async(&mut connection)
                .await
                .map_err(store_error)?;
            Ok(JobId(id))
        })
    }

    fn claim<'a>(
        &'a self,
        now: DateTime<Utc>,
        lease: Duration,
        limit: usize,
    ) -> StoreFuture<'a, Vec<Job>> {
        Box::pin(async move {
            let mut connection = self.redis.connection().await?;
            let script = Script::new(&format!(
                "{NOW}
                 local limit = tonumber(ARGV[3])
                 local out = redis.call('ZRANGEBYSCORE', KEYS[1], '-inf', ARGV[1], 'LIMIT', 0, limit)
                 if #out < limit then
                   local expired = redis.call('ZRANGEBYSCORE', KEYS[2], '-inf', '(' .. now,
                                              'LIMIT', 0, limit - #out)
                   for _, id in ipairs(expired) do table.insert(out, id) end
                 end
                 for _, id in ipairs(out) do
                   local job = ARGV[4] .. ':job:' .. id
                   redis.call('ZREM', KEYS[1], id)
                   redis.call('HSET', job, 'status', 'running')
                   redis.call('HINCRBY', job, 'attempts', 1)
                   redis.call('ZADD', KEYS[2], now + tonumber(ARGV[2]), id)
                 end
                 return out"
            ));
            let ids: Vec<String> = script
                .key(QUEUE)
                .key(RUNNING)
                .arg(millis(now))
                .arg(lease.as_millis() as i64)
                .arg(limit)
                .arg(PREFIX)
                .invoke_async(&mut connection)
                .await
                .map_err(store_error)?;
            let mut jobs = Vec::with_capacity(ids.len());
            for id in ids {
                if let Some(job) = self.load(&mut connection, &id).await? {
                    jobs.push(job);
                }
            }
            Ok(jobs)
        })
    }

    fn heartbeat<'a>(&'a self, id: &'a JobId, lease_until: DateTime<Utc>) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let mut connection = self.redis.connection().await?;
            let script = Script::new(
                "if redis.call('HGET', KEYS[1], 'status') == 'running' then
                   redis.call('ZADD', KEYS[2], ARGV[2], ARGV[1])
                 end
                 return 1",
            );
            let _: i64 = script
                .key(key("job", &id.0))
                .key(RUNNING)
                .arg(&id.0)
                .arg(millis(lease_until))
                .invoke_async(&mut connection)
                .await
                .map_err(store_error)?;
            Ok(())
        })
    }

    fn extend_lease<'a>(
        &'a self,
        id: &'a JobId,
        _now: DateTime<Utc>,
        lease: Duration,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let mut connection = self.redis.connection().await?;
            let script = Script::new(&format!(
                "{NOW}
                 if redis.call('HGET', KEYS[1], 'status') == 'running' then
                   redis.call('ZADD', KEYS[2], now + tonumber(ARGV[2]), ARGV[1])
                 end
                 return 1"
            ));
            let _: i64 = script
                .key(key("job", &id.0))
                .key(RUNNING)
                .arg(&id.0)
                .arg(lease.as_millis() as i64)
                .invoke_async(&mut connection)
                .await
                .map_err(store_error)?;
            Ok(())
        })
    }

    fn finish<'a>(&'a self, id: &'a JobId, outcome: JobOutcome) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let mut connection = self.redis.connection().await?;
            let Some(mut job) = self.load(&mut connection, &id.0).await? else {
                return Ok(());
            };
            job.lease_until = None;
            match outcome {
                JobOutcome::Succeeded(output) => {
                    job.status = JobStatus::Succeeded;
                    job.last_error = None;
                    job.result = Some(output);
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
            let script = Script::new(&format!(
                "{NOW}
                 redis.call('ZREM', KEYS[3], ARGV[1])
                 redis.call('HSET', KEYS[1], 'json', ARGV[2], 'status', ARGV[3])
                 if ARGV[3] == 'queued' then
                   redis.call('ZADD', KEYS[2], ARGV[4], ARGV[1])
                   return 1
                 end
                 if redis.call('GET', KEYS[6]) == ARGV[1] then redis.call('DEL', KEYS[6]) end
                 if ARGV[3] == 'dead_lettered' then
                   redis.call('ZADD', KEYS[4], ARGV[4], ARGV[1])
                 else
                   redis.call('ZADD', KEYS[5], now, ARGV[1])
                 end
                 return 1"
            ));
            let json = serde_json::to_string(&job).map_err(store_error)?;
            let _: i64 = script
                .key(key("job", &id.0))
                .key(QUEUE)
                .key(RUNNING)
                .key(DEAD)
                .key(FINISHED)
                .key(key(
                    "active",
                    job.request.request_id.as_deref().unwrap_or(""),
                ))
                .arg(&id.0)
                .arg(json)
                .arg(job.status.as_str())
                .arg(millis(job.run_at))
                .invoke_async(&mut connection)
                .await
                .map_err(store_error)?;
            Ok(())
        })
    }

    fn cancel<'a>(&'a self, id: &'a JobId) -> StoreFuture<'a, bool> {
        Box::pin(async move {
            let mut connection = self.redis.connection().await?;
            let request_id = self
                .load(&mut connection, &id.0)
                .await?
                .and_then(|job| job.request.request_id);
            let script = Script::new(&format!(
                "{NOW}
                 if redis.call('HGET', KEYS[1], 'status') ~= 'queued' then return 0 end
                 redis.call('HSET', KEYS[1], 'status', 'cancelled')
                 redis.call('ZREM', KEYS[2], ARGV[1])
                 redis.call('ZADD', KEYS[3], now, ARGV[1])
                 if redis.call('GET', KEYS[4]) == ARGV[1] then redis.call('DEL', KEYS[4]) end
                 return 1"
            ));
            let done: i64 = script
                .key(key("job", &id.0))
                .key(QUEUE)
                .key(FINISHED)
                .key(key("active", request_id.as_deref().unwrap_or("")))
                .arg(&id.0)
                .invoke_async(&mut connection)
                .await
                .map_err(store_error)?;
            Ok(done == 1)
        })
    }

    fn get<'a>(&'a self, id: &'a JobId) -> StoreFuture<'a, Option<Job>> {
        Box::pin(async move {
            let mut connection = self.redis.connection().await?;
            self.load(&mut connection, &id.0).await
        })
    }

    fn dead_letters<'a>(&'a self, limit: usize) -> StoreFuture<'a, Vec<Job>> {
        Box::pin(async move {
            let mut connection = self.redis.connection().await?;
            let ids: Vec<String> = connection
                .zrange(DEAD, 0, limit.max(1) as isize - 1)
                .await
                .map_err(store_error)?;
            let mut jobs = Vec::with_capacity(ids.len());
            for id in ids {
                if let Some(job) = self.load(&mut connection, &id).await? {
                    jobs.push(job);
                }
            }
            Ok(jobs)
        })
    }

    fn purge<'a>(&'a self, before: DateTime<Utc>) -> StoreFuture<'a, u64> {
        Box::pin(async move {
            let mut connection = self.redis.connection().await?;
            // In batches, so no single script blocks Redis for long.
            let script = Script::new(
                "local ids = redis.call('ZRANGEBYSCORE', KEYS[1], '-inf', '(' .. ARGV[1],
                                        'LIMIT', 0, 1000)
                 for _, id in ipairs(ids) do
                   redis.call('DEL', ARGV[2] .. ':job:' .. id)
                   redis.call('ZREM', KEYS[1], id)
                 end
                 return #ids",
            );
            let mut total = 0u64;
            loop {
                let deleted: u64 = script
                    .key(FINISHED)
                    .arg(millis(before))
                    .arg(PREFIX)
                    .invoke_async(&mut connection)
                    .await
                    .map_err(store_error)?;
                total += deleted;
                if deleted == 0 {
                    return Ok(total);
                }
            }
        })
    }
}

// ---------------------------------------------------------------- idempotency

/// [`IdempotencyStore`] on Redis, shared by every instance of a service.
/// Reservations are atomic; completed records expire after the retention, and
/// in-progress ones never do.
#[derive(Clone)]
pub struct RedisIdempotencyStore {
    redis: Redis,
    retention: Duration,
}

impl RedisIdempotencyStore {
    /// Completed records expire after seven days.
    pub fn new(redis: Redis) -> Self {
        Self {
            redis,
            retention: Duration::from_secs(7 * 24 * 3600),
        }
    }
    /// How long a completed record replays. A `request_id` retried after this runs
    /// again, so keep it longer than any client retries. Zero keeps records forever.
    pub fn with_retention(mut self, retention: Duration) -> Self {
        self.retention = retention;
        self
    }
}

fn idempotency_key(key: &IdempotencyKey) -> String {
    let identity = serde_json::to_string(&(&key.principal, &key.session, &key.request_id))
        .expect("keys serialize");
    self::key("idem", &identity)
}

impl IdempotencyStore for RedisIdempotencyStore {
    fn reserve<'a>(
        &'a self,
        key: &'a IdempotencyKey,
        fingerprint: &'a str,
    ) -> StoreFuture<'a, Reservation> {
        Box::pin(async move {
            let mut connection = self.redis.connection().await?;
            let script = Script::new(
                "local fp = redis.call('HGET', KEYS[1], 'fingerprint')
                 if not fp then
                   redis.call('HSET', KEYS[1], 'fingerprint', ARGV[1])
                   return {'acquired'}
                 end
                 if fp ~= ARGV[1] then return {'conflict'} end
                 local result = redis.call('HGET', KEYS[1], 'result')
                 if result then return {'replay', result} end
                 return {'in_progress'}",
            );
            let answer: Vec<String> = script
                .key(idempotency_key(key))
                .arg(fingerprint)
                .invoke_async(&mut connection)
                .await
                .map_err(store_error)?;
            Ok(match answer.first().map(String::as_str) {
                Some("acquired") => Reservation::Acquired,
                Some("conflict") => Reservation::Conflict,
                Some("replay") => {
                    let stored = answer.get(1).map(String::as_str).unwrap_or("null");
                    let result: ExecutionResult =
                        serde_json::from_str(stored).map_err(store_error)?;
                    Reservation::Replay(result)
                }
                _ => Reservation::InProgress,
            })
        })
    }

    fn complete<'a>(
        &'a self,
        key: &'a IdempotencyKey,
        fingerprint: &'a str,
        result: &'a ExecutionResult,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let mut connection = self.redis.connection().await?;
            let script = Script::new(
                "if redis.call('HGET', KEYS[1], 'fingerprint') ~= ARGV[1]
                    or redis.call('HEXISTS', KEYS[1], 'result') == 1 then
                   return 0
                 end
                 redis.call('HSET', KEYS[1], 'result', ARGV[2])
                 if tonumber(ARGV[3]) > 0 then redis.call('PEXPIRE', KEYS[1], ARGV[3]) end
                 return 1",
            );
            let stored = serde_json::to_string(result).map_err(store_error)?;
            let done: i64 = script
                .key(idempotency_key(key))
                .arg(fingerprint)
                .arg(stored)
                .arg(self.retention.as_millis() as i64)
                .invoke_async(&mut connection)
                .await
                .map_err(store_error)?;
            if done == 1 {
                Ok(())
            } else {
                Err(AgentError::new(
                    "IDEMPOTENCY_CONFLICT",
                    "Reservation no longer matches",
                    ErrorCategory::Conflict,
                ))
            }
        })
    }
}
