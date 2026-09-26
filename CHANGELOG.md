# Changelog

All notable changes to Carmy are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/). APIs are unstable during 0.x.

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

[0.2.0]: https://github.com/igorvieira/Carmy/releases/tag/v0.2.0
[0.1.0]: https://github.com/igorvieira/Carmy/releases/tag/v0.1.0
