<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/igorvieira/Carmy/main/assets/brand/lockup-dark.png">
    <img alt="Carmy: Composable Agent Runtime for Managed Yield" src="https://raw.githubusercontent.com/igorvieira/Carmy/main/assets/brand/lockup.png" width="420">
  </picture>
</p>

<p align="center">
  <a href="https://github.com/igorvieira/Carmy/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/igorvieira/Carmy/actions/workflows/ci.yml/badge.svg?branch=main"></a>
  <a href="https://crates.io/crates/carmy"><img alt="crates.io" src="https://img.shields.io/crates/v/carmy?color=B23A2B&logo=rust"></a>
  <a href="https://docs.rs/carmy"><img alt="docs.rs" src="https://img.shields.io/docsrs/carmy?logo=docsdotrs"></a>
  <a href="https://www.rust-lang.org"><img alt="Built with Rust" src="https://img.shields.io/badge/built%20with-Rust-B23A2B?logo=rust&logoColor=white"></a>
  <a href="https://carmy-pi.vercel.app"><img alt="Documentation" src="https://img.shields.io/badge/docs-carmy--pi.vercel.app-0E0F10?logo=astro&logoColor=white"></a>
  <a href="https://modelcontextprotocol.io"><img alt="MCP" src="https://img.shields.io/badge/MCP-supported-6B7280"></a>
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-6B7280"></a>
  <a href="https://github.com/sponsors/igorvieira"><img alt="Sponsor" src="https://img.shields.io/badge/sponsor-%E2%99%A5-B23A2B?logo=githubsponsors&logoColor=white"></a>
</p>

# Carmy

Agent-native execution infrastructure for Rust.

**Documentation:** [carmy-pi.vercel.app](https://carmy-pi.vercel.app), in English and Brazilian Portuguese.

Carmy is **not** another general-purpose HTTP framework. It does not route web pages or
compete with Axum, Actix or Hyper; it uses them. Carmy is the execution layer between AI
agents and real systems: typed tools with explicit side effects, machine-readable errors,
safe retries, cancellation and tracing, exposed through replaceable transports (HTTP and
MCP today).

> LLM reasoning can be probabilistic. Side effects should not be.

Status: `0.2.0` on [crates.io](https://crates.io/crates/carmy). The API is unstable during 0.x.

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

## Getting started

```console
$ cargo install carmy-cli        # requires Rust 1.88+
$ carmy new shop && cd shop
$ carmy g tool search --effect read   # add a tool: file, test and registration
$ cargo run              # HTTP on http://127.0.0.1:3000/.well-known/agent
$ cargo run -- mcp       # the same app as an MCP server over stdio
$ cargo run -- tools     # print the tool catalog
$ cargo test
```

Carmy favors convention over configuration. A new application looks like this:

```text
shop/
├── Cargo.toml
├── carmy.toml          # name, address, timeout; CARMY_* env vars override
└── src/
    ├── main.rs         # carmy::run().await
    └── tools/
        ├── mod.rs      # mod hello;  (one line per tool)
        └── hello.rs    # the tool and its tests
```

A tool is an `async fn` with an attribute:

```rust
// src/tools/search.rs
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
async fn search(input: SearchInput) -> AgentResult<SearchOutput> {
    Ok(SearchOutput {
        results: vec![format!("Result for {}", input.query)],
    })
}
```

Generate it with `carmy g tool search --effect read`, which writes the file with a test
and declares it in `src/tools/mod.rs`, or add `mod search;` there yourself. The tool
registers itself, and agents discover it over HTTP and MCP:

```console
$ curl localhost:3000/agent/execute -H 'content-type: application/json' \
    -d '{"tool":"search","arguments":{"query":"mechanical keyboard"}}'
{"execution_id":"exec_…","status":"completed","data":{"results":["Result for mechanical keyboard"]},"_agent":{"cacheable":true,"next_actions":[]}}
```

Test a tool next to its code, through the same runtime pipeline that agents use:

```rust
use carmy::serde_json::json;

#[tokio::test]
async fn searches() {
    let result = carmy::testing::execute(search, json!({ "query": "x" })).await;
    assert_eq!(result.outcome.unwrap()["results"][0], "Result for x");
}
```

### Dependencies: `State<T>`

Register shared dependencies once, then ask for them by type:

```rust
#[carmy::tool(description = "Place an order", effect = "write")]
async fn create_order(State(db): State<Db>, input: NewOrder) -> AgentResult<Order> {
    db.insert(input).await
}

#[tokio::main]
async fn main() -> carmy::Result {
    let db = Db::connect("postgres://localhost/shop").await.expect("database");
    carmy::app().state(db).run().await
}
```

A missing dependency is reported at startup (`MISSING_STATE: tool create_order requires
State<shop::Db>`), never on a request. `State` values are cloned per execution, so pass
`Arc`s, pools or clients.

### What is automatic, and how to opt out

| convention                                       | explicit alternative |
|--------------------------------------------------|----------------------|
| `#[carmy::tool]` registers the tool in `carmy::app()` | `#[carmy::tool(register = false)]` and `.tool(x)` |
| `carmy::app()` reads `carmy.toml` and `CARMY_*`  | `Carmy::new()`, with no file, env or auto-registration |
| `run()` chooses `server`, `mcp` or `tools` from the first argument | `.listen(addr)`, `.serve_mcp_stdio()`, `.build()` |
| tracing to stderr                                | `default-features = false` and your own subscriber |

Auto-registration collects tools at link time (via `linkme`), so it only covers crates
linked into the binary. `.tool(x)` works everywhere.

Runnable examples in this repository:

- `cargo run -p tool-server`: the conventional style, with `State`. Add `-- mcp` to serve it over MCP.
- `cargo run -p hello-agent`: the explicit layer. It drives the runtime in-process, with no transport.

## Tools

Everything below is what the conventions build on.

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

`#[carmy::tool]` generates this for an `async fn` that returns `AgentResult<Output>` and
takes any of the following, in any order:

- an optional `AgentContext`
- any number of `State<T>`
- at most one input. A tool with no input accepts `{}`.

The generated type is a unit struct named after the function. Macro attributes:

| attribute       | values                                                         | default   |
|-----------------|----------------------------------------------------------------|-----------|
| `description`   | string                                                         | `""`      |
| `effect`        | `none`, `read`, `write`, `external_write`, `destructive`       | required  |
| `idempotent`    | bool                                                           | `false`   |
| `parallel_safe` | bool (`false` serializes calls to this tool)                   | `false`   |
| `confirmation`  | `none`, `required`                                             | `none`    |
| `register`      | bool (auto-registration in `carmy::app()`)                     | `true`    |

A malformed signature, an unknown attribute, or a type without `JsonSchema` fails at
compile time with a pointed error. These cases are covered by `trybuild` tests.

`AgentContext` holds only framework data: execution and request IDs, session, principal,
permissions, metadata and the cancellation token. Application dependencies go in
`State<T>`, or in a struct that implements `Tool` directly. The context is never a
service locator.

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
the tool's `_meta`. See [the MCP page](https://carmy-pi.vercel.app/transports/mcp/) for the full mapping and what v0.1 does
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
| `carmy`               | facade: `carmy::app()`, `State`, config, `testing`, `prelude`, feature-gated transports |
| `carmy-cli`           | `carmy new`                                                           |
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

More detail is in [Architecture](https://carmy-pi.vercel.app/reference/architecture/), and the HTTP wire format is
in [HTTP](https://carmy-pi.vercel.app/transports/http/).

## Observability

Each execution runs in a `carmy.execution` tracing span with these fields:

- `execution_id`, `request_id`, `tool` and `effect`
- `status`, `duration_ms` and `replayed`
- `error_code`, when the execution fails

Arguments and outputs are never recorded. `carmy::run()` installs a stderr subscriber
(`carmy_observability::init()`). For OpenTelemetry, compose `carmy_observability::fmt_layer()` with a
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

The same tool, served to the same client ([method and raw results](benches/compare)):

| MCP over stdio | p50 | throughput | peak memory | cold start |
|---|---|---|---|---|
| **Carmy** | 45 µs | 87,100 req/s | 11 MB | 4 ms |
| TypeScript SDK | 52 µs | 72,600 req/s | 219 MB | 135 ms |
| Python SDK | 456 µs | 3,100 req/s | 65 MB | 351 ms |

Over HTTP, Carmy's guarantees add about 3–4 µs at p50 over a plain Axum handler. Apple M3
Pro; reproduce with `benches/compare/run.sh`. The full tables, including where Carmy
loses and what changed since 0.1.0, are on the
[benchmarks page](https://carmy-pi.vercel.app/reference/benchmarks/#comparisons).

`cargo bench -p carmy-benches` measures Carmy's internal overhead. This project makes no
performance claims that these benchmarks cannot reproduce.

## Roadmap

- **Execution plans:** DAGs of tool calls with `$step.field` references, built on today's `ExecutionRequest` and runtime.
- **Tool progress events:** progress and partial results emitted from tools into the event stream.
- **Transports:** MCP Streamable HTTP, and a `next_actions` vocabulary.
- **Idempotency:** durable `IdempotencyStore` adapters as separate crates.
- **Publishing:** a first crates.io release.

Carmy will not add its own async runtime, HTTP parser, TLS stack, ORM, workflow engine,
agent memory, LLM abstraction or prompt framework.

## Sponsor

Carmy is independent, MIT-licensed open source. Sponsorship pays for maintenance time,
new transports and adapters, documentation in English and Portuguese, and keeping
releases and CI healthy.

**[Sponsor Carmy on GitHub →](https://github.com/sponsors/igorvieira)**

Companies that sponsor Carmy get their logo in this README and on the
[documentation site](https://carmy-pi.vercel.app/sponsor/).

## License

MIT
