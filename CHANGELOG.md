# Changelog

All notable changes to Carmy are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/). APIs are unstable during 0.x.

## [Unreleased]

## [0.5.0] - 2026-09-27

### Added

- **Agents follow async work.** Every app registers `carmy_job`, a read-only tool that
  answers a job's status, attempts, next run and last error from the `job_id` an
  enqueue returned, never its arguments. It works over HTTP, MCP and the console alike
  and goes through the same policies as any tool. `Carmy::operator_tools()` adds
  `carmy_dead_letters` and `carmy_audit`. The `carmy_` names are reserved.
- **Postgres from configuration.** `[database] url` in `carmy.toml`,
  `CARMY_DATABASE_URL` or `DATABASE_URL` (feature `postgres`), or `Carmy::database(url)`,
  moves jobs, idempotency and audit to Postgres. The pool connects on first use. Tools
  can take `State<PgPool>` and `State<PostgresJobStore>`. `run()` migrates before any
  command but `tools`. The `database` readiness check and the `migrate` and `cleanup`
  commands come with it, and retention is configurable. A URL without the feature is a
  configuration error.
- `carmy_postgres::connect_lazy`.
- New apps get commented `[jobs]` and `[database]` sections in `carmy.toml`.

### Changed

- `Carmy::command` takes a plain async closure: `|app| async move { .. }`. Closures
  that return `Box::pin(..)` still compile.
- `PostgresAudit` starts its writer on the first record, so an app can be built outside
  a Tokio runtime.
- The roadmap and README state the limits of jobs precisely. Jobs run one tool call
  later; they are not a workflow engine or a cluster scheduler.
- The curator reads its database from `DATABASE_URL` and no longer wires Postgres by
  hand.

### Fixed

- **A built app no longer leaks its runtime.** Tools holding the job queue
  (`State<Jobs>`), and the queue holding the runtime, formed a reference cycle that was
  never freed. The facade now binds the queue with `Jobs::bind_weak`: whoever runs the
  app keeps the runtime, and a queue that outlives it fails with `JOBS_UNBOUND`. The
  fuzzer's leak sanitizer found it.

## [0.4.1] - 2026-09-26

### Fixed

- **`carmy_postgres::migrate` no longer shares `_sqlx_migrations` with the
  application.** Carmy tracks its versions in `carmy_schema_version`, so an app with its
  own sqlx migrations no longer collides with Carmy's. Instances migrating at once take
  turns on an advisory lock, and `migrate` can run inside `tokio::spawn`. Databases set
  up by 0.4.0 upgrade in place.
- **Leases use the database clock.** `PostgresJobStore` sets and checks leases with
  `now()` on the database, so workers with skewed clocks never take a live job. The new
  `JobStore::extend_lease` (with a default) lets any store do the same.
- **The in-memory job store no longer fills up with finished jobs.** When full, it drops
  finished jobs before refusing new ones; queued, running and dead-lettered jobs are
  never dropped.

### Added

- **Retention.** `carmy_postgres::cleanup(pool, Retention)` deletes, in batches, finished
  jobs (30 days), completed idempotency records (7 days) and audit records (90 days);
  dead letters and in-progress records always stay. `Jobs::purge(older_than)` and
  `JobStore::purge` do the job part for any store. The curator has a `cleanup` command.
- A Postgres guide, in English and Portuguese.

## [0.4.0] - 2026-09-26

### Added

- **`carmy-jobs`: tools that run later.** `Jobs` enqueues an `ExecutionRequest` now,
  after a delay or at a time, and `every(name, cron, make)` schedules it. A job's
  `request_id` is its identity, so enqueueing twice never runs twice. Retries follow the
  error: `retryable` errors back off exponentially (respecting `retry_after`) up to
  `max_attempts`; uncertain outcomes (`TIMEOUT`, `CANCELLED`, `TOOL_PANIC`) retry only
  idempotent tools and otherwise dead-letter with `EXECUTION_UNCERTAIN`. `cargo run --
  worker` runs jobs with leases, heartbeats and graceful shutdown; `[jobs]` in
  `carmy.toml` sets `concurrency` and `max_attempts`; tools take `State<Jobs>`.
- **`carmy-postgres`** (feature `postgres`): `PostgresJobStore` (claims with `FOR UPDATE
  SKIP LOCKED`, a transactional outbox through `enqueue_in`), `PostgresIdempotencyStore`
  (atomic reservations shared across instances) and `PostgresAudit`, plus `migrate` and
  `ready`. The contract tests run against a Postgres service in CI.
- **Webhooks, a door into a tool.** `Carmy::webhook(path, Webhook::to(tool)
  .verify(check))` checks each delivery with any function of its headers and raw body
  (`verify::hmac_sha256` and `verify::shared_secret` come ready), takes the
  `request_id` from a JSON pointer into the payload (`.event_id("/id")`) so redeliveries
  replay, and runs the tool inline or as a job (`.enqueue()`, answering `202`). A
  webhook without a check refuses to start unless it says `.unverified()`. Discovery
  and the console's `webhooks` list them without secrets. Carmy ships no
  sender-specific integrations.
- **Audit trail.** `ExecutionSink` receives an `ExecutionRecord` after every execution,
  replays and rejections included, never with arguments or outputs. Every app keeps the
  latest records in memory; `.sink(..)` adds durable ones. The console gains `audit` and
  `dead`.
- **Readiness, routes and commands.** `GET /health`, `GET /ready` (every `.ready(name,
  check)` together, with a timeout, `503` naming what failed, `.require_worker(within)`
  for the queue), `Carmy::routes(axum::Router)` behind the same hardening,
  `Carmy::command(name, run)` for the app's own commands, and `Carmy::router_and_jobs`.
- **`examples/curator`,** the reference pipeline application: a scheduled collection,
  one job per offer, publications that never duplicate, a signed billing webhook, `/ready`,
  `/deals`, and Postgres behind `DATABASE_URL`. Its end-to-end test runs two workers, a
  refused publication and a redelivered webhook.

### Security

- **Connection-level protections** in `ServerOptions`, on by default: a header timeout
  that also closes idle keep-alive connections (10 s), a body timeout (30 s), and a
  connection limit (4096). Each has a contract test over a real socket. Configure them in
  `carmy.toml` under `[http]`, with `CARMY_HTTP_*` variables, or with `Carmy::http`.
- **Opt-in security headers and CORS** (`security_headers`, `cors`).
- **`RateLimit`**, a ready-made `ExecutionPolicy`: a fixed window per principal, with
  `RATE_LIMITED` errors that carry `retry_after`.
- **`cargo deny` in CI:** known vulnerabilities, licenses, sources and wildcard
  dependencies.
- **Fuzzing in CI** with `cargo fuzz`, on the console protocol and `POST /agent/execute`.

## [0.3.0] - 2026-09-26

### Added

- `carmy generate tool <name> --effect <effect>` (and `carmy g tool`) writes
  `src/tools/<name>.rs` and declares it in `src/tools/mod.rs`. The file has typed input
  and output, the `#[carmy::tool]` function and a test, with attributes that follow the
  effect. It never overwrites a file and runs from any directory inside the application.
- **Console.** `cargo run -- console` serves `carmy-console/1`, a JSON Lines protocol
  for agents and scripts. It supports `tools`, `describe`, `call` (with `request_id`),
  and `confirm`/`revoke` scoped to the session, plus text shortcuts. Every call goes
  through the runtime with all its guarantees. `carmy::console::serve` embeds it.
- **`carmy console`,** a `ratatui` terminal UI over that protocol:
  - effect badges and confirmation locks;
  - arguments prefilled from the input schema;
  - `request_id` replays;
  - a confirmation modal and history.

  With `--jsonl`, or without a terminal, it passes the protocol through.
- `carmy server` runs the application over HTTP from any directory inside it.

## [0.2.0] - 2026-09-26

### Performance

Every change came from profiling under the comparison load, and each kept every
guarantee. Measured with `benches/compare` on an Apple M3 Pro:

- **MCP stdio:** served through readiness-driven pipes when stdin and stdout are pipes.
  It falls back to blocking stdio for TTYs, files and Windows.
- **Runtime hot path:**
  - The idempotency fingerprint is computed only for requests with a `request_id`, and
    streamed into the hash without copying the arguments.
  - Execution IDs no longer make a system call per request.
  - Lifecycle events are built only when a stream is listening.
  - The deadline timer and the cancellation waiter are registered only if the tool
    suspends. The precedence of cancellation and deadline is unchanged.
- **HTTP:**
  - The handler reads `Accept` without cloning every header.
  - A cancellation child token is created only for a host-provided context.
- **MCP:** arguments are moved instead of cloned, and result metadata is built without
  intermediate copies.

Result: MCP throughput went from 38,900 to 87,100 req/s (+124%), and HTTP on one thread
from 69,600 to 92,300 req/s (+33%). Over HTTP, the p50 cost of Carmy's guarantees fell
from about 7 µs to about 3–4 µs.

### Changed

- `_meta["carmy/execution_id"]` and `_meta["carmy/status"]` on **successful** MCP
  results are now opt-in, with `McpServer::execution_meta(true)`. Errors still always
  carry them, along with `_meta["carmy/error"]`.

### Added

- `guarantee/*` Criterion benchmarks, measuring the cost of each guarantee on its own.
- Tests for panic isolation on both execution paths, for deadlines on suspended tools,
  for execution ID uniqueness, and for stdio over pipes and over files.

### Fixed

- The comparison harness now disables per-call logging for Carmy over MCP too.

## [0.1.0] - 2026-09-26

The first release.

### Added

- **Typed tools** with `#[carmy::tool]`, with JSON Schemas derived from input and output types.
- **Explicit effects** (`none`, `read`, `write`, `external_write`, `destructive`) and trusted confirmation for destructive tools.
- **Machine-readable errors:** code, category, `recoverable`, `retryable`, `retry_after`, `suggested_action` and `details`.
- **The runtime:**
  - schema validation of inputs and outputs
  - execution policies
  - deadlines and cancellation tokens
  - panic isolation
  - per-tool serialization
- **Idempotency:** atomic `request_id` reservation, replay of recorded results, conflict and uncertainty detection, and a pluggable `IdempotencyStore`.
- **Streaming:** a runtime event stream, served as SSE over HTTP.
- **HTTP transport:**
  - `/.well-known/agent` and `/agent/tools`, with ETags
  - `/agent/execute`
- **MCP transport** on the official `rmcp` SDK: `tools/list`, `tools/call`, cancellation, effect annotations and `carmy/request_id`.
- **Conventions:**
  - `carmy::app()` and `carmy::run()`
  - auto-registered tools
  - `State<T>` injection
  - `carmy.toml`
  - `carmy::testing`
- **`carmy new`,** which creates a conventional application.
- **Tracing spans** for every execution, without recording payloads.
- **Documentation** at https://carmy-pi.vercel.app, in English and Brazilian Portuguese.

[0.5.0]: https://github.com/igorvieira/Carmy/releases/tag/v0.5.0
[0.4.1]: https://github.com/igorvieira/Carmy/releases/tag/v0.4.1
[0.4.0]: https://github.com/igorvieira/Carmy/releases/tag/v0.4.0
[0.3.0]: https://github.com/igorvieira/Carmy/releases/tag/v0.3.0
[0.2.0]: https://github.com/igorvieira/Carmy/releases/tag/v0.2.0
[0.1.0]: https://github.com/igorvieira/Carmy/releases/tag/v0.1.0
