-- Job results: what a succeeded job's tool returned.
ALTER TABLE carmy_jobs ADD COLUMN IF NOT EXISTS result JSONB;
