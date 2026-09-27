//! A database from a URL: Postgres or Redis, chosen by the URL's scheme.
use std::time::Duration;

/// How long `cleanup` keeps finished work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    /// Jobs that succeeded, failed or were cancelled. Dead letters always stay.
    pub jobs: Duration,
    /// Completed idempotency records. A `request_id` retried after this runs again, so
    /// keep it longer than any client retries. In-progress records always stay.
    pub idempotency: Duration,
    /// Audit records (Postgres; with Redis the audit trail stays in memory).
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

#[cfg(feature = "postgres")]
impl From<carmy_postgres::Retention> for Retention {
    fn from(r: carmy_postgres::Retention) -> Self {
        Self {
            jobs: r.jobs,
            idempotency: r.idempotency,
            audit: r.audit,
        }
    }
}
#[cfg(feature = "postgres")]
impl From<Retention> for carmy_postgres::Retention {
    fn from(r: Retention) -> Self {
        Self {
            jobs: r.jobs,
            idempotency: r.idempotency,
            audit: r.audit,
        }
    }
}

/// Which store keeps the app's durable state.
#[derive(Clone)]
pub(crate) enum Backend {
    #[cfg(feature = "postgres")]
    Postgres(carmy_postgres::sqlx::PgPool),
    #[cfg(feature = "redis")]
    Redis(Box<carmy_redis::Redis>),
}

impl Backend {
    /// Open `url` without connecting. The scheme picks the store.
    pub(crate) fn open(url: &str) -> Result<Self, String> {
        let scheme = url.split("://").next().unwrap_or_default();
        match scheme {
            "postgres" | "postgresql" => {
                #[cfg(feature = "postgres")]
                return carmy_postgres::connect_lazy(url)
                    .map(Backend::Postgres)
                    .map_err(|e| format!("database url: {e}"));
                #[cfg(not(feature = "postgres"))]
                return Err("a postgres:// database needs carmy's `postgres` feature".into());
            }
            "redis" | "rediss" => {
                #[cfg(feature = "redis")]
                return carmy_redis::Redis::open(url)
                    .map(|redis| Backend::Redis(Box::new(redis)))
                    .map_err(|e| format!("database url: {e}"));
                #[cfg(not(feature = "redis"))]
                return Err("a redis:// database needs carmy's `redis` feature".into());
            }
            _ => Err("database url must start with postgres:// or redis://".into()),
        }
    }

    /// Create or update Carmy's tables. Redis has nothing to migrate.
    pub(crate) async fn migrate(&self) -> Result<(), String> {
        match self {
            #[cfg(feature = "postgres")]
            Backend::Postgres(pool) => carmy_postgres::migrate(pool)
                .await
                .map_err(|e| e.to_string()),
            #[cfg(feature = "redis")]
            Backend::Redis(_) => Ok(()),
        }
    }

    /// Delete what `retention` no longer keeps; returns what was deleted, as JSON.
    pub(crate) async fn cleanup(&self, retention: Retention) -> Result<serde_json::Value, String> {
        match self {
            #[cfg(feature = "postgres")]
            Backend::Postgres(pool) => {
                let cleaned = carmy_postgres::cleanup(pool, retention.into())
                    .await
                    .map_err(|e| e.message)?;
                Ok(serde_json::json!({
                    "jobs": cleaned.jobs,
                    "idempotency": cleaned.idempotency,
                    "audit": cleaned.audit,
                }))
            }
            // Idempotency records expire on their own; the audit trail is in memory.
            #[cfg(feature = "redis")]
            Backend::Redis(redis) => {
                use carmy_jobs::JobStore;
                let redis: &carmy_redis::Redis = redis;
                let age = chrono::Duration::from_std(retention.jobs).map_err(|e| e.to_string())?;
                let jobs = carmy_redis::RedisJobStore::new(redis.clone())
                    .purge(chrono::Utc::now() - age)
                    .await
                    .map_err(|e| e.message)?;
                Ok(serde_json::json!({ "jobs": jobs }))
            }
        }
    }
}
