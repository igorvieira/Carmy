//! Durable stores on Postgres: jobs (with a transactional outbox), idempotency and audit.
//!
//! ```no_run
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let pool = carmy_postgres::connect("postgres://localhost/shop").await?;
//! carmy_postgres::migrate(&pool).await?;
//! let jobs = std::sync::Arc::new(carmy_postgres::PostgresJobStore::new(pool.clone()));
//! let idempotency = std::sync::Arc::new(carmy_postgres::PostgresIdempotencyStore::new(pool.clone()));
//! let audit = std::sync::Arc::new(carmy_postgres::PostgresAudit::new(pool));
//! # Ok(()) }
//! ```
//!
//! Every `carmy_*` table is created by [`migrate`], which is safe to run on every start
//! and never touches the application's own migrations. [`cleanup`] deletes old rows.
//! Leases use the database clock, so workers on machines with skewed clocks agree on
//! when a job's owner is gone.
use carmy_core::{AgentError, AgentResult, ErrorCategory, ExecutionRequest, ExecutionResult};
use carmy_jobs::{Job, JobId, JobOutcome, JobRequest, JobStatus, JobStore, StoreFuture};
use carmy_runtime::{
    ExecutionRecord, ExecutionSink, IdempotencyKey, IdempotencyStore, Reservation,
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgRow};
use std::time::Duration;

pub use sqlx;

/// A pool with sensible defaults for a Carmy service.
pub async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(5))
        .connect(url)
        .await
}

/// [`connect`] without connecting: the pool opens connections on first use, so it can
/// be built synchronously, e.g. from configuration. Fails only on an invalid URL.
pub fn connect_lazy(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(5))
        .connect_lazy(url)
}

/// Carmy's schema, in order. Versions are tracked in `carmy_schema_version`, apart from
/// `_sqlx_migrations`, so they never collide with the application's migrations.
const MIGRATIONS: &[(i32, &str)] = &[
    (1, include_str!("../migrations/0001_carmy.sql")),
    (2, include_str!("../migrations/0002_audit.sql")),
    (3, include_str!("../migrations/0003_retention.sql")),
    (4, include_str!("../migrations/0004_job_results.sql")),
];

/// Creates or updates the `carmy_*` tables. Idempotent, and safe to run from several
/// instances at once: they take turns on an advisory lock.
pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    use sqlx::migrate::MigrateError::Execute;
    let mut tx = pool.begin().await.map_err(Execute)?;
    apply_migrations(&mut tx).await.map_err(Execute)?;
    tx.commit().await.map_err(Execute)
}

/// Boxed as `Send` here, with concrete lifetimes, so `migrate` can run inside
/// `tokio::spawn` (sqlx's generic executors otherwise trip the compiler's
/// higher-ranked lifetime checks).
fn apply_migrations<'c>(
    conn: &'c mut sqlx::PgConnection,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), sqlx::Error>> + Send + 'c>> {
    Box::pin(async move {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('carmy_schema'))")
            .execute(&mut *conn)
            .await?;
        // Checked first: `CREATE ... IF NOT EXISTS` would log a notice on every start.
        let exists: bool =
            sqlx::query_scalar("SELECT to_regclass('carmy_schema_version') IS NOT NULL")
                .fetch_one(&mut *conn)
                .await?;
        if !exists {
            sqlx::query(
                "CREATE TABLE carmy_schema_version ( \
                   version INTEGER PRIMARY KEY, \
                   applied_at TIMESTAMPTZ NOT NULL DEFAULT now() \
                 )",
            )
            .execute(&mut *conn)
            .await?;
        }
        let applied: Vec<i32> = sqlx::query_scalar("SELECT version FROM carmy_schema_version")
            .fetch_all(&mut *conn)
            .await?;
        for (version, sql) in MIGRATIONS {
            if applied.contains(version) {
                continue;
            }
            // Through the `Executor` trait: its boxed future is `Send`, while
            // `RawSql::execute`'s is not general enough to cross `tokio::spawn`.
            sqlx::Executor::execute(&mut *conn, sqlx::raw_sql(sql)).await?;
            sqlx::query("INSERT INTO carmy_schema_version (version) VALUES ($1)")
                .bind(version)
                .execute(&mut *conn)
                .await?;
        }
        Ok(())
    })
}

/// How long [`cleanup`] keeps finished work.
#[derive(Debug, Clone, Copy)]
pub struct Retention {
    /// Jobs that succeeded, failed or were cancelled. Dead letters always stay.
    pub jobs: Duration,
    /// Completed idempotency records. A `request_id` retried after this runs again, so
    /// keep it longer than any client retries. In-progress records always stay.
    pub idempotency: Duration,
    /// Audit records.
    pub audit: Duration,
}

impl Default for Retention {
    /// 30 days of jobs, 7 days of idempotency, 90 days of audit.
    fn default() -> Self {
        const DAY: u64 = 24 * 3600;
        Self {
            jobs: Duration::from_secs(30 * DAY),
            idempotency: Duration::from_secs(7 * DAY),
            audit: Duration::from_secs(90 * DAY),
        }
    }
}

/// Rows [`cleanup`] deleted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cleaned {
    pub jobs: u64,
    pub idempotency: u64,
    pub audit: u64,
}

/// Delete what [`Retention`] no longer keeps, in batches so no table stays locked for
/// long. Ages use the database clock. Run it from a command or a schedule.
pub async fn cleanup(pool: &PgPool, retention: Retention) -> AgentResult<Cleaned> {
    let batch = |table: &str, condition: &str| {
        format!(
            "DELETE FROM {table} WHERE ctid IN ( \
               SELECT ctid FROM {table} WHERE {condition} LIMIT 5000 \
             )"
        )
    };
    let delete = |sql: String, age: Duration| async move {
        let mut total = 0;
        loop {
            let done = sqlx::query(&sql)
                .bind(age.as_secs_f64())
                .execute(pool)
                .await
                .map_err(store_error)?
                .rows_affected();
            total += done;
            if done == 0 {
                return Ok::<u64, AgentError>(total);
            }
        }
    };
    let older = "< now() - make_interval(secs => $1)";
    Ok(Cleaned {
        jobs: delete(
            batch(
                "carmy_jobs",
                &format!(
                    "status IN ('succeeded', 'failed', 'cancelled') \
                     AND COALESCE(finished_at, run_at) {older}"
                ),
            ),
            retention.jobs,
        )
        .await?,
        idempotency: delete(
            batch(
                "carmy_idempotency",
                &format!("result IS NOT NULL AND created_at {older}"),
            ),
            retention.idempotency,
        )
        .await?,
        audit: delete(
            batch("carmy_audit", &format!("started_at {older}")),
            retention.audit,
        )
        .await?,
    })
}

/// A readiness check for the pool: `SELECT 1` must answer. Pass it to
/// `Carmy::ready("database", move || ready(pool.clone()))`.
pub async fn ready(pool: PgPool) -> AgentResult<()> {
    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .map(|_| ())
        .map_err(|e| {
            AgentError::new(
                "DATABASE_UNAVAILABLE",
                format!("Postgres did not answer: {e}"),
                ErrorCategory::Capacity,
            )
            .retryable(Some(1))
        })
}

fn store_error(e: sqlx::Error) -> AgentError {
    AgentError::new(
        "STORE_ERROR",
        format!("Postgres store failed: {e}"),
        ErrorCategory::Internal,
    )
    .retryable(Some(1))
}

// ---------------------------------------------------------------- jobs

/// [`JobStore`] on Postgres. Claims use `FOR UPDATE SKIP LOCKED`, so any number of
/// workers can share a queue.
#[derive(Clone)]
pub struct PostgresJobStore {
    pool: PgPool,
}

impl PostgresJobStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The transactional outbox: enqueue `request` inside the caller's transaction,
    /// so the job exists exactly when the surrounding write commits.
    pub async fn enqueue_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        request: ExecutionRequest,
        run_at: DateTime<Utc>,
        max_attempts: u32,
    ) -> AgentResult<JobId> {
        let job = carmy_jobs::Jobs::prepare(request, run_at, max_attempts);
        insert(&mut **tx, &job).await
    }
}

const COLUMNS: &str = "id, tool, arguments, request_id, metadata, principal, session, permissions, \
                       context_metadata, run_at, created_at, attempts, max_attempts, status, \
                       last_error, lease_until, schedule, result";

async fn insert<'e, E>(executor: E, job: &Job) -> AgentResult<JobId>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    // One statement: insert, or return the active job that already carries this request_id.
    let row: PgRow = sqlx::query(
        "WITH inserted AS ( \
           INSERT INTO carmy_jobs (id, tool, arguments, request_id, metadata, principal, session, \
             permissions, context_metadata, run_at, created_at, attempts, max_attempts, status, \
             last_error, lease_until, schedule) \
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17) \
           ON CONFLICT (request_id) WHERE status IN ('queued', 'running') DO NOTHING \
           RETURNING id \
         ) \
         SELECT id FROM inserted \
         UNION ALL \
         SELECT id FROM carmy_jobs \
         WHERE request_id = $4 AND status IN ('queued', 'running') \
           AND NOT EXISTS (SELECT 1 FROM inserted) \
         LIMIT 1",
    )
    .bind(&job.id.0)
    .bind(&job.request.tool)
    .bind(&job.request.arguments)
    .bind(&job.request.request_id)
    .bind(Value::Object(
        job.request.metadata.clone().into_iter().collect(),
    ))
    .bind(&job.request.principal)
    .bind(&job.request.session)
    .bind(Value::Array(
        job.request
            .permissions
            .iter()
            .map(|p| Value::String(p.clone()))
            .collect(),
    ))
    .bind(Value::Object(
        job.request.context_metadata.clone().into_iter().collect(),
    ))
    .bind(job.run_at)
    .bind(job.created_at)
    .bind(job.attempts as i32)
    .bind(job.max_attempts as i32)
    .bind(job.status.as_str())
    .bind(
        job.last_error
            .as_ref()
            .map(|e| serde_json::to_value(e).expect("errors serialize")),
    )
    .bind(job.lease_until)
    .bind(&job.schedule)
    .fetch_one(executor)
    .await
    .map_err(store_error)?;
    Ok(JobId(row.get("id")))
}

fn job_from_row(row: &PgRow) -> AgentResult<Job> {
    let object = |value: Value| match value {
        Value::Object(map) => map.into_iter().collect(),
        _ => Default::default(),
    };
    let status = match row.get::<String, _>("status").as_str() {
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
    };
    let permissions = match row.get::<Value, _>("permissions") {
        Value::Array(items) => items
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        _ => Default::default(),
    };
    Ok(Job {
        id: JobId(row.get("id")),
        request: JobRequest {
            tool: row.get("tool"),
            arguments: row.get("arguments"),
            request_id: row.get("request_id"),
            metadata: object(row.get("metadata")),
            principal: row.get("principal"),
            session: row.get("session"),
            permissions,
            context_metadata: object(row.get("context_metadata")),
        },
        run_at: row.get("run_at"),
        created_at: row.get("created_at"),
        attempts: row.get::<i32, _>("attempts").max(0) as u32,
        max_attempts: row.get::<i32, _>("max_attempts").max(0) as u32,
        status,
        last_error: row
            .get::<Option<Value>, _>("last_error")
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| AgentError::new("STORE_ERROR", e.to_string(), ErrorCategory::Internal))?,
        lease_until: row.get("lease_until"),
        schedule: row.get("schedule"),
        result: row.get("result"),
    })
}

impl JobStore for PostgresJobStore {
    fn enqueue<'a>(&'a self, job: Job) -> StoreFuture<'a, JobId> {
        Box::pin(async move { insert(&self.pool, &job).await })
    }

    fn claim<'a>(
        &'a self,
        now: DateTime<Utc>,
        lease: Duration,
        limit: usize,
    ) -> StoreFuture<'a, Vec<Job>> {
        Box::pin(async move {
            // Due times follow the app's clock (`run_at` was set by it); leases follow
            // the database's, so every worker agrees on when an owner is gone.
            let rows = sqlx::query(&format!(
                "WITH due AS ( \
                   SELECT id FROM carmy_jobs \
                   WHERE (status = 'queued' AND run_at <= $1) \
                      OR (status = 'running' AND lease_until < now()) \
                   ORDER BY run_at \
                   LIMIT $2 \
                   FOR UPDATE SKIP LOCKED \
                 ) \
                 UPDATE carmy_jobs AS j \
                 SET status = 'running', attempts = j.attempts + 1, \
                     lease_until = now() + make_interval(secs => $3) \
                 FROM due WHERE j.id = due.id \
                 RETURNING j.id, {}",
                COLUMNS.trim_start_matches("id, ")
            ))
            .bind(now)
            .bind(limit as i64)
            .bind(lease.as_secs_f64())
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?;
            rows.iter().map(job_from_row).collect()
        })
    }

    fn heartbeat<'a>(&'a self, id: &'a JobId, lease_until: DateTime<Utc>) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE carmy_jobs SET lease_until = $1 WHERE id = $2 AND status = 'running'",
            )
            .bind(lease_until)
            .bind(&id.0)
            .execute(&self.pool)
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
            sqlx::query(
                "UPDATE carmy_jobs SET lease_until = now() + make_interval(secs => $1) \
                 WHERE id = $2 AND status = 'running'",
            )
            .bind(lease.as_secs_f64())
            .bind(&id.0)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn purge<'a>(&'a self, before: DateTime<Utc>) -> StoreFuture<'a, u64> {
        Box::pin(async move {
            let done = sqlx::query(
                "DELETE FROM carmy_jobs WHERE status IN ('succeeded', 'failed', 'cancelled') \
                 AND COALESCE(finished_at, run_at) < $1",
            )
            .bind(before)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            Ok(done.rows_affected())
        })
    }

    fn finish<'a>(&'a self, id: &'a JobId, outcome: JobOutcome) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let error = |e: &AgentError| serde_json::to_value(e).expect("errors serialize");
            let result = match &outcome {
                JobOutcome::Succeeded(output) => Some(output.clone()),
                _ => None,
            };
            let (status, run_at, last_error) = match &outcome {
                JobOutcome::Succeeded(_) => ("succeeded", None, None),
                JobOutcome::Retry { at, error: e } => ("queued", Some(*at), Some(error(e))),
                JobOutcome::Failed(e) => ("failed", None, Some(error(e))),
                JobOutcome::DeadLettered(e) => ("dead_lettered", None, Some(error(e))),
            };
            sqlx::query(
                "UPDATE carmy_jobs SET status = $1, run_at = COALESCE($2, run_at), \
                 last_error = $3, lease_until = NULL, \
                 finished_at = CASE WHEN $1 = 'queued' THEN NULL ELSE now() END, \
                 result = $5 \
                 WHERE id = $4",
            )
            .bind(status)
            .bind(run_at)
            .bind(last_error)
            .bind(&id.0)
            .bind(result)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn cancel<'a>(&'a self, id: &'a JobId) -> StoreFuture<'a, bool> {
        Box::pin(async move {
            let done = sqlx::query(
                "UPDATE carmy_jobs SET status = 'cancelled', finished_at = now() \
                 WHERE id = $1 AND status = 'queued'",
            )
            .bind(&id.0)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            Ok(done.rows_affected() == 1)
        })
    }

    fn get<'a>(&'a self, id: &'a JobId) -> StoreFuture<'a, Option<Job>> {
        Box::pin(async move {
            let row = sqlx::query(&format!("SELECT {COLUMNS} FROM carmy_jobs WHERE id = $1"))
                .bind(&id.0)
                .fetch_optional(&self.pool)
                .await
                .map_err(store_error)?;
            row.as_ref().map(job_from_row).transpose()
        })
    }

    fn dead_letters<'a>(&'a self, limit: usize) -> StoreFuture<'a, Vec<Job>> {
        Box::pin(async move {
            let rows = sqlx::query(&format!(
                "SELECT {COLUMNS} FROM carmy_jobs WHERE status = 'dead_lettered' \
                 ORDER BY run_at LIMIT $1"
            ))
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?;
            rows.iter().map(job_from_row).collect()
        })
    }
}

// ---------------------------------------------------------------- idempotency

/// [`IdempotencyStore`] on Postgres, shared by every instance of a service. Reservations
/// are atomic (`INSERT ... ON CONFLICT`), and in-progress rows are never evicted.
#[derive(Clone)]
pub struct PostgresIdempotencyStore {
    pool: PgPool,
}

impl PostgresIdempotencyStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn key_text(key: &IdempotencyKey) -> String {
    serde_json::to_string(&(&key.principal, &key.session, &key.request_id)).expect("keys serialize")
}

impl IdempotencyStore for PostgresIdempotencyStore {
    fn reserve<'a>(
        &'a self,
        key: &'a IdempotencyKey,
        fingerprint: &'a str,
    ) -> StoreFuture<'a, Reservation> {
        Box::pin(async move {
            let key = key_text(key);
            let inserted = sqlx::query(
                "INSERT INTO carmy_idempotency (key, fingerprint) VALUES ($1, $2) \
                 ON CONFLICT (key) DO NOTHING",
            )
            .bind(&key)
            .bind(fingerprint)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            if inserted.rows_affected() == 1 {
                return Ok(Reservation::Acquired);
            }
            let row =
                sqlx::query("SELECT fingerprint, result FROM carmy_idempotency WHERE key = $1")
                    .bind(&key)
                    .fetch_one(&self.pool)
                    .await
                    .map_err(store_error)?;
            if row.get::<String, _>("fingerprint") != fingerprint {
                return Ok(Reservation::Conflict);
            }
            match row.get::<Option<Value>, _>("result") {
                Some(result) => {
                    let result: ExecutionResult = serde_json::from_value(result).map_err(|e| {
                        AgentError::new("STORE_ERROR", e.to_string(), ErrorCategory::Internal)
                    })?;
                    Ok(Reservation::Replay(result))
                }
                None => Ok(Reservation::InProgress),
            }
        })
    }

    fn complete<'a>(
        &'a self,
        key: &'a IdempotencyKey,
        fingerprint: &'a str,
        result: &'a ExecutionResult,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let done = sqlx::query(
                "UPDATE carmy_idempotency SET result = $1 \
                 WHERE key = $2 AND fingerprint = $3 AND result IS NULL",
            )
            .bind(serde_json::to_value(result).expect("results serialize"))
            .bind(key_text(key))
            .bind(fingerprint)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            if done.rows_affected() == 1 {
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

// ---------------------------------------------------------------- audit

/// [`ExecutionSink`] on Postgres. Records go through a bounded channel to one writer
/// task, so executions never wait on the database; when the channel is full, the
/// record is dropped and a warning logged rather than slowing the runtime.
pub struct PostgresAudit {
    pool: PgPool,
    capacity: usize,
    /// Started on the first record, so building an app needs no running runtime.
    writer: std::sync::OnceLock<(
        tokio::sync::mpsc::Sender<ExecutionRecord>,
        tokio::task::JoinHandle<()>,
    )>,
}

impl PostgresAudit {
    /// Buffers up to 4096 records.
    pub fn new(pool: PgPool) -> Self {
        Self::with_capacity(pool, 4096)
    }

    pub fn with_capacity(pool: PgPool, capacity: usize) -> Self {
        Self {
            pool,
            capacity: capacity.max(1),
            writer: std::sync::OnceLock::new(),
        }
    }

    fn sender(&self) -> &tokio::sync::mpsc::Sender<ExecutionRecord> {
        &self
            .writer
            .get_or_init(|| {
                let (sender, mut receiver) = tokio::sync::mpsc::channel(self.capacity);
                let pool = self.pool.clone();
                let writer = tokio::spawn(async move {
                    while let Some(record) = receiver.recv().await {
                        if let Err(e) = insert_record(&pool, &record).await {
                            tracing::warn!(
                                execution_id = %record.execution_id,
                                error = %e,
                                "audit record was not written"
                            );
                        }
                    }
                });
                (sender, writer)
            })
            .0
    }

    /// Up to `limit` records, newest first.
    pub async fn recent(&self, limit: usize) -> AgentResult<Vec<ExecutionRecord>> {
        let rows = sqlx::query(
            "SELECT execution_id, request_id, principal, session, tool, effect, status, \
             error_code, duration_ms, started_at, replayed \
             FROM carmy_audit ORDER BY started_at DESC, recorded_at DESC LIMIT $1",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        rows.iter().map(record_from_row).collect()
    }

    /// Stop accepting records and wait until every buffered one is written.
    pub async fn flush(self) {
        if let Some((sender, writer)) = self.writer.into_inner() {
            drop(sender);
            let _ = writer.await;
        }
    }
}

impl ExecutionSink for PostgresAudit {
    /// Runs on the execution's task, inside the runtime.
    fn record(&self, record: ExecutionRecord) {
        if let Err(e) = self.sender().try_send(record) {
            tracing::warn!(
                execution_id = %e.into_inner().execution_id,
                "audit buffer is full; record dropped"
            );
        }
    }
}

async fn insert_record(pool: &PgPool, record: &ExecutionRecord) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO carmy_audit (execution_id, request_id, principal, session, tool, effect, \
         status, error_code, duration_ms, started_at, replayed) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(&record.execution_id)
    .bind(&record.request_id)
    .bind(&record.principal)
    .bind(&record.session)
    .bind(&record.tool)
    .bind(record.effect.map(|e| e.as_str()))
    .bind(record.status.as_str())
    .bind(&record.error_code)
    .bind(record.duration_ms as i64)
    .bind(record.started_at)
    .bind(record.replayed)
    .execute(pool)
    .await?;
    Ok(())
}

fn record_from_row(row: &PgRow) -> AgentResult<ExecutionRecord> {
    let text = |name: &str| row.get::<String, _>(name);
    let effect = row
        .get::<Option<String>, _>("effect")
        .map(|e| serde_json::from_value(Value::String(e)))
        .transpose()
        .map_err(|e| AgentError::new("STORE_ERROR", e.to_string(), ErrorCategory::Internal))?;
    let status = serde_json::from_value(Value::String(text("status")))
        .map_err(|e| AgentError::new("STORE_ERROR", e.to_string(), ErrorCategory::Internal))?;
    Ok(ExecutionRecord {
        execution_id: text("execution_id"),
        request_id: row.get("request_id"),
        principal: row.get("principal"),
        session: row.get("session"),
        tool: text("tool"),
        effect,
        status,
        error_code: row.get("error_code"),
        duration_ms: row.get::<i64, _>("duration_ms").max(0) as u64,
        started_at: row.get("started_at"),
        replayed: row.get("replayed"),
    })
}
