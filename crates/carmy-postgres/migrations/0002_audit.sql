-- Audit: one row per execution, replays included. Never arguments or outputs.
CREATE TABLE IF NOT EXISTS carmy_audit (
    execution_id TEXT NOT NULL,
    request_id   TEXT,
    principal    TEXT,
    session      TEXT,
    tool         TEXT NOT NULL,
    effect       TEXT,
    status       TEXT NOT NULL,
    error_code   TEXT,
    duration_ms  BIGINT NOT NULL,
    started_at   TIMESTAMPTZ NOT NULL,
    replayed     BOOLEAN NOT NULL DEFAULT FALSE,
    recorded_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS carmy_audit_started_at ON carmy_audit (started_at DESC);
CREATE INDEX IF NOT EXISTS carmy_audit_principal ON carmy_audit (principal, started_at DESC);
