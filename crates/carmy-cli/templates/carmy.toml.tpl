# Carmy configuration. Environment variables override these values.
name = "{{name}}"            # CARMY_NAME
address = "127.0.0.1:3000"   # CARMY_ADDR
timeout_secs = 30            # CARMY_TIMEOUT_SECS

# Jobs run with `cargo run -- worker`.
# [jobs]
# concurrency = 4            # CARMY_JOBS_CONCURRENCY
# max_attempts = 5           # CARMY_JOBS_MAX_ATTEMPTS

# Durable jobs, idempotency and audit. Needs `features = ["postgres"]` on carmy.
# [database]
# url = "postgres://localhost/{{name}}"   # CARMY_DATABASE_URL or DATABASE_URL
