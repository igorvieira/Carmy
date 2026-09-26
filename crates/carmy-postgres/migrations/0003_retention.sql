-- Retention: when a job finished, and indexes for deleting old rows in batches.
ALTER TABLE carmy_jobs ADD COLUMN IF NOT EXISTS finished_at TIMESTAMPTZ;
CREATE INDEX IF NOT EXISTS carmy_jobs_finished
    ON carmy_jobs (finished_at) WHERE status IN ('succeeded', 'failed', 'cancelled');
CREATE INDEX IF NOT EXISTS carmy_idempotency_created ON carmy_idempotency (created_at);
