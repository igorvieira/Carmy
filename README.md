# Carmy

Agent-native execution infrastructure for Rust.

Carmy is **not** another general-purpose HTTP framework. It does not route web pages or
compete with Axum, Actix or Hyper; it uses them. Carmy is the execution layer between AI
agents and real systems: typed tools with explicit side effects, machine-readable errors,
safe retries, cancellation and tracing, exposed through replaceable transports (HTTP and
MCP today).

> LLM reasoning can be probabilistic. Side effects should not be.

Status: `0.1.0`, unstable API, not yet published to crates.io.

## Why Carmy

Agents call tools in unusual ways. They retry after timeouts without knowing whether the
first attempt committed. They read tool catalogs instead of documentation. They act on
error codes rather than prose. They abandon requests midway. Carmy makes those cases
explicit:

- **Tools are typed Rust functions** with JSON Schemas derived from their input and output types.
- **Effects are declared**, never inferred from HTTP methods or names, so clients can apply approval policies before execution.
- **Errors are data**: code, category, `recoverable`, `retryable`, `retry_after`, `suggested_action`.
- **Retries are safe**: a repeated `request_id` replays the recorded result instead of executing again.
- **Cancellation is real**: timeouts, client disconnects and MCP cancellations reach the tool's cancellation token.
- **Transports are adapters**: HTTP and MCP drive the same runtime and the same tool code.

## Quick start

```toml
[dependencies]
carmy = { git = "https://github.com/igorvieira/carmy" }
serde = { version = "1", features = ["derive"] }
schemars = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust
use carmy::prelude::*;

#[derive(Deserialize, JsonSchema)]
struct SearchInput {
    /// Search expression.
    query: String,
}

#[derive(Serialize, JsonSchema)]
struct SearchOutput {
    results: Vec<String>,
}

#[carmy::tool(description = "Search the catalog", effect = "read", idempotent = true)]
async fn search(_ctx: AgentContext, input: SearchInput) -> AgentResult<SearchOutput> {
    Ok(SearchOutput {
        results: vec![format!("Result for {}", input.query)],
    })
}

#[tokio::main]
async fn main() -> Result<(), carmy::Error> {
    Carmy::new().tool(search).listen("0.0.0.0:3000").await
}
```

```console
$ curl localhost:3000/.well-known/agent
{"capabilities":["tools","streaming","idempotency"],"execute_url":"/agent/execute","protocol":"carmy/1","server":"carmy","tools_url":"/agent/tools","tools_version":"3762…"}

$ curl localhost:3000/agent/execute -H 'content-type: application/json' \
    -d '{"tool":"search","arguments":{"query":"mechanical keyboard"}}'
{"execution_id":"exec_…","status":"completed","data":{"results":["Result for mechanical keyboard"]},"_agent":{"cacheable":true,"next_actions":[]}}
```

To serve the same tools over MCP (stdio), enable the `mcp` feature and call
`Carmy::new().tool(search).serve_mcp_stdio().await`.

Runnable examples:

- `cargo run -p hello-agent`: drives the runtime in-process, with no transport.
- `cargo run -p tool-server`: serves the same tools over HTTP. Add `-- --mcp` to serve them over MCP stdio.

## Tools

A tool implements `carmy::Tool`:

```rust
pub trait Tool: Send + Sync + 'static {
    type Input: DeserializeOwned + JsonSchema + Send + 'static;
    type Output: Serialize + JsonSchema + Send + 'static;
    fn metadata(&self) -> ToolMetadata;
    fn execute(&self, ctx: AgentContext, input: Self::Input)
        -> impl Future<Output = AgentResult<Self::Output>> + Send;
}
```

`#[carmy::tool]` generates this for an `async fn(AgentContext, Input) -> AgentResult<Output>`.
The generated type is a unit struct named after the function. Its metadata holds the name,
description, input and output schemas, `effect`, `idempotent`, `parallel_safe` and
`confirmation`. Macro attributes:

| attribute       | values                                                         | default   |
|-----------------|----------------------------------------------------------------|-----------|
| `description`   | string                                                         | `""`      |
| `effect`        | `none`, `read`, `write`, `external_write`, `destructive`       | required  |
| `idempotent`    | bool                                                           | `false`   |
| `parallel_safe` | bool (`false` serializes calls to this tool)                   | `false`   |
| `confirmation`  | `none`, `required`                                             | `none`    |

A malformed signature, an unknown attribute, or a type without `JsonSchema` fails at
compile time with a pointed error. These cases are covered by `trybuild` tests.

**Application state** belongs to the tool value, not the framework context. Implement
`Tool` on a struct that holds your dependencies (see `examples/tool-server`).
`AgentContext` holds only framework data: execution and request IDs, session, principal,
permissions, metadata and the cancellation token. It is not a service locator.

## Effects

```rust
pub enum Effect { None, Read, Write, ExternalWrite, Destructive }
```

The effect of every tool is declared and published in discovery, so clients and hosts can
decide on approval before calling it. `Destructive` tools and tools declaring
`confirmation = "required"` are rejected by the default policy (`CONFIRMATION_REQUIRED`)
unless the host puts `confirm:<tool>` into the trusted context's permissions. Tool
arguments can never grant this.

## Errors

Errors are designed for machines first:

```json
{
  "error": {
    "code": "PRODUCT_NOT_FOUND",
    "message": "No product has this SKU",
    "category": "not_found",
    "recoverable": true,
    "retryable": false,
    "suggested_action": "search_products"
  }
}
```

```rust
AgentError::new("PRODUCT_NOT_FOUND", "No product has this SKU", ErrorCategory::NotFound)
    .recoverable()
    .suggest("search_products")
```

Categories: `validation`, `not_found`, `permission`, `conflict`, `cancelled`, `timeout`,
`internal`, `capacity`. The runtime produces stable codes of its own:

- `TOOL_NOT_FOUND`, `INVALID_ARGUMENTS`, `INVALID_OUTPUT`
- `CONFIRMATION_REQUIRED`
- `IDEMPOTENCY_CONFLICT`, `EXECUTION_UNCERTAIN`
- `TIMEOUT`, `CANCELLED`, `TOOL_PANIC`

Only the HTTP adapter maps categories to status codes; the core has no notion of HTTP.

## Idempotency

A request carrying a `request_id` is reserved atomically before the tool runs. Its key is
the principal, the session and the `request_id`. The runtime also fingerprints the tool
name, arguments and metadata.

| situation                                | outcome                                       |
|------------------------------------------|-----------------------------------------------|
| same identity, same fingerprint, finished | recorded result replayed; the tool does **not** run again |
| same identity, different fingerprint      | `IDEMPOTENCY_CONFLICT`                        |
| same identity while running or interrupted | `EXECUTION_UNCERTAIN`: reconcile, don't blindly retry |

Timeouts and cancellations are recorded as results too. External effects may already have
committed, so a retry never silently repeats them. `InMemoryIdempotencyStore` is bounded
and fails closed when full. Implement `IdempotencyStore` to add durable storage, such as
Redis or Postgres.

## Streaming

The runtime exposes an execution event stream, `Runtime::execute_stream`, independent of
any wire format:

`execution.started` → `tool.started` → `tool.completed` → `execution.completed`

Replays and early rejections skip the `tool.*` events. SSE is only the HTTP encoding: send
`Accept: text/event-stream` to `POST /agent/execute`. The stream drives the execution
itself. Dropping it, for example when the client disconnects, cancels the execution just
as dropping a normal request does.

## Cancellation and timeouts

Every execution has a `CancellationToken` in its context and a runtime deadline (default
30 s, set with `Carmy::timeout`). The runtime cancels the token and stops polling the tool
in these cases:

- the deadline passes
- the HTTP client disconnects (JSON or SSE)
- an MCP `notifications/cancelled` arrives for the call
- the caller drops the execution future

Background work spawned by a tool should watch `ctx.cancellation`. Interrupted executions
keep their idempotency reservation, so the uncertainty stays visible.

## MCP

`carmy-mcp` adapts the runtime to the Model Context Protocol using the official `rmcp`
SDK. It supports `initialize`, `ping`, `tools/list`, `tools/call` and request
cancellation. Effects map to MCP tool annotations, and the exact Carmy metadata is kept in
the tool's `_meta`. See [docs/mcp.md](docs/mcp.md) for the full mapping and what v0.1 does
not support.

## Architecture

```text
                  Agent
                    │
          ┌─────────┴─────────┐
          │                   │
        HTTP                 MCP          transports (carmy-http, carmy-mcp)
          │                   │
          └─────────┬─────────┘
                    │  ExecutionRequest
               Carmy Runtime              resolution · policy · validation ·
                    │                     idempotency · deadline · tracing
          ┌─────────┼─────────┐
          │         │         │
        Tool      Tool      Tool          carmy-core domain
          │         │         │
       Service   Database  External API
```

| crate                 | role                                                                  |
|-----------------------|-----------------------------------------------------------------------|
| `carmy`               | facade: `Carmy` builder, `prelude`, feature-gated transports          |
| `carmy-core`          | domain: `Tool`, `ToolMetadata`, `Effect`, `AgentError`, `AgentContext`, execution types |
| `carmy-schema`        | JSON Schema generation                                                |
| `carmy-macros`        | `#[carmy::tool]`                                                      |
| `carmy-runtime`       | registry, policies, validation, idempotency, cancellation, event stream |
| `carmy-http`          | discovery, tool catalog, execution and SSE over Axum                  |
| `carmy-mcp`           | MCP server adapter over `rmcp`                                        |
| `carmy-observability` | tracing subscriber setup and OpenTelemetry composition                |

The invariants:

- The core does not know about transports, and tools do not know about HTTP.
- The runtime does not know about LLM providers.
- Effects are explicit, and errors are machine-readable.
- Retries are safe, and execution is observable.
- Transports are replaceable.

More detail is in [docs/architecture.md](docs/architecture.md), and the HTTP wire format is
in [docs/protocol.md](docs/protocol.md).

## Observability

Each execution runs in a `carmy.execution` tracing span with these fields:

- `execution_id`, `request_id`, `tool` and `effect`
- `status`, `duration_ms` and `replayed`
- `error_code`, when the execution fails

Arguments and outputs are never recorded. `carmy_observability::init()` installs a stderr
subscriber. For OpenTelemetry, compose `carmy_observability::fmt_layer()` with a
`tracing-opentelemetry` layer; Carmy itself does not depend on OpenTelemetry.

## Security

Carmy provides small hooks rather than a policy framework:

- **Authentication:** host middleware inserts a trusted `AgentContext` (HTTP `Extension`, `McpServer::context`). Request bodies cannot set context.
- **Authorization and tool permissions:** `ExecutionPolicy` hooks run before every execution, including replays. `RequireToolPermission` is one example.
- **Effect and confirmation policies:** the default `SafePolicy` gates destructive and confirmation-required tools.
- **Input limits:** JSON Schema validation runs before execution, with a 1 MiB HTTP body limit, bounded request IDs and bounded idempotency storage.
- **Timeouts:** a per-runtime execution deadline.
- **Rate limiting:** implement it as an `ExecutionPolicy` (returning `ErrorCategory::Capacity`) or as Tower middleware on the router.

Destructive tools are identifiable in discovery, before anything executes.

## Response efficiency

The discovery and catalog documents are serialized once at startup and served with an
`ETag` and `Cache-Control`. Discovery includes `tools_version`, so an agent can skip
refetching an unchanged catalog, and `If-None-Match` returns `304`. MCP `tools/list` sets
a 60 s `ttlMs`. Execution responses omit empty fields, and
`_agent.cacheable` tells clients when a result can be reused: successful, idempotent,
side-effect-free tools.

## Benchmarks

```console
cargo bench -p carmy-benches
```

The benchmarks measure runtime dispatch against a direct tool call, result serialization,
schema and catalog generation, the in-process HTTP round trip, and 64 concurrent read-only
executions. The fixture tool does no work, so the numbers show Carmy's own overhead. See
[docs/benchmarks.md](docs/benchmarks.md). This project makes no performance claims that
these benchmarks cannot reproduce.

## Roadmap

- **Execution plans:** DAGs of tool calls with `$step.field` references, built on today's `ExecutionRequest` and runtime.
- **Tool progress events:** progress and partial results emitted from tools into the event stream.
- **Transports:** MCP Streamable HTTP, and a `next_actions` vocabulary.
- **Idempotency:** durable `IdempotencyStore` adapters as separate crates.
- **Publishing:** a first crates.io release.

Carmy will not add its own async runtime, HTTP parser, TLS stack, ORM, workflow engine,
agent memory, LLM abstraction or prompt framework.

## License

MIT
