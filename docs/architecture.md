# Architecture

Carmy separates the **domain**, the **runtime** and the **transports**, so the same tools
can be run under different transports.

```text
carmy-core ◄── carmy-runtime ◄── carmy-http
    ▲               ▲        ◄── carmy-mcp
    │               │        ◄── (future transports)
carmy-schema    carmy-observability (subscriber setup only)
carmy-macros
```

`carmy` is the facade that applications depend on.

## Domain: `carmy-core`

`Tool`, `ToolMetadata`, `Effect`, `Confirmation`, `AgentError`, `ErrorCategory`,
`AgentContext`, `ExecutionRequest`, `ExecutionResult`, `ExecutionStatus` and
`ExecutionEvent` live here. The core has no dependencies on HTTP, MCP, Axum, Hyper,
databases or LLM providers. Its dependencies are serde, serde_json and schemars (for tool
schemas), plus tokio-util (for `CancellationToken`).

`Tool` uses an associated `Input` and `Output` type and a return-position `impl Future`.
Tool code is statically dispatched and needs no `async-trait` boxing. The runtime erases
tool types once, at registration: one boxed future per execution, behind a
`BTreeMap<String, _>` lookup.

`AgentContext` is framework context only. Application dependencies live in the tool value
itself (`struct CreateOrder(Arc<Orders>)`), so they stay explicit and typed.

## Runtime: `carmy-runtime`

`Runtime::execute(ExecutionRequest) -> ExecutionResult` runs this pipeline:

1. Resolve the tool by name (`TOOL_NOT_FOUND`).
2. Run the `ExecutionPolicy` hooks: authorization, confirmation, quotas. They run on replays too.
3. Validate IDs, and validate the arguments against the input schema, which is compiled once at registration.
4. Reserve the idempotency identity: replay, conflict, uncertain, or acquire.
5. Invoke the tool under three conditions:
   - a runtime deadline
   - the cancellation token
   - panic isolation
6. Validate the output against the output schema.
7. Record the result in the idempotency store.
8. Close the tracing span with the status, duration, replay flag and error code.

`execute_stream` runs the same pipeline and yields `ExecutionEvent`s. The stream owns and
polls the execution future directly, with no spawned task. Dropping the stream therefore
has the same cancellation semantics as dropping `execute`: the execution's token is
cancelled, and its idempotency reservation stays in the uncertain state.

Tools that are not `parallel_safe` are serialized per tool with an async mutex.

## Transports

A transport has three jobs:

1. Translate its protocol into an `ExecutionRequest`, with a fresh execution ID and the host's trusted `AgentContext`.
2. Link its own cancellation signal to the context's token.
3. Translate `ExecutionResult` or `ExecutionEvent` into its own DTOs.

Runtime types never go on the wire directly. HTTP has `ResultDto`, `ToolDto` and SSE
payloads. MCP has `Tool`, `CallToolResult` and annotations.

## Review checklist

Check these for every change:

- Is the concept part of the domain, or part of a transport?
- Does HTTP or MCP leak into the runtime or the core? Status codes, headers and JSON-RPC stay in adapters.
- Does application state leak into `AgentContext`?
- Is a new abstraction needed now, or only allowed by the architecture?
- Does it prevent execution plans or a new transport later?
- Does it make the common case harder?

## Future execution plans

Plans (DAGs of steps with `$step.field` references) can be built on top of
`Runtime::execute`. Each step is an ordinary `ExecutionRequest`, and each step result is
an `ExecutionResult`. The metadata needed for scheduling already exists:

- `parallel_safe` decides which steps may run concurrently.
- `effect` and `idempotent` decide which steps may be retried or cached.

A plan identity can derive per-step `request_id`s (`<plan>/<step>`), so retrying a partly
failed plan only replays or resumes what is needed. None of this requires changes to tools
or transports.
