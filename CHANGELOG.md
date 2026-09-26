# Changelog

All notable changes to Carmy are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/). APIs are unstable during 0.x.

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

[0.1.0]: https://github.com/igorvieira/Carmy/releases/tag/v0.1.0
