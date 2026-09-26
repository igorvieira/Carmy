-- Jobs: tools that run later. `request_id` is unique among jobs that have not finished,
-- which makes enqueueing idempotent.
CREATE TABLE IF NOT EXISTS carmy_jobs (
    id               TEXT PRIMARY KEY,
    tool             TEXT NOT NULL,
    arguments        JSONB NOT NULL,
    request_id       TEXT,
    metadata         JSONB NOT NULL DEFAULT '{}'::jsonb,
    principal        TEXT,
    session          TEXT,
    permissions      JSONB NOT NULL DEFAULT '[]'::jsonb,
    context_metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    run_at           TIMESTAMPTZ NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL,
    attempts         INTEGER NOT NULL DEFAULT 0,
    max_attempts     INTEGER NOT NULL,
    status           TEXT NOT NULL,
    last_error       JSONB,
    lease_until      TIMESTAMPTZ,
    schedule         TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS carmy_jobs_active_request_id
    ON carmy_jobs (request_id) WHERE status IN ('queued', 'running');
CREATE INDEX IF NOT EXISTS carmy_jobs_due ON carmy_jobs (status, run_at);
CREATE INDEX IF NOT EXISTS carmy_jobs_lease ON carmy_jobs (lease_until) WHERE status = 'running';

-- Idempotency: one row per (principal, session, request_id); `result` is null while
-- the execution is running or was interrupted, and is never evicted.
CREATE TABLE IF NOT EXISTS carmy_idempotency (
    key         TEXT PRIMARY KEY,
    fingerprint TEXT NOT NULL,
    result      JSONB,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
