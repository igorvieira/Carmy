---
title: Architecture
description: "The crates, the execution pipeline and the boundaries between them."
sidebar:
  order: 1
---

```text
                  Agent
                    │
          ┌─────────┴─────────┐
        HTTP                 MCP          transports (carmy-http, carmy-mcp)
          └─────────┬─────────┘
                    │  ExecutionRequest
               Carmy Runtime              policy · validation · idempotency ·
                    │                     deadline · tracing
          ┌─────────┼─────────┐
        Tool      Tool      Tool          your code
          │         │         │
       Service   Database  External API
```

## Crates

| crate | role |
|-------|------|
| `carmy` | the facade: `carmy::app()`, `State`, config, `testing`, `prelude`, and feature-gated transports |
| `carmy-core` | the domain: `Tool`, `ToolMetadata`, `Effect`, `AgentError`, `AgentContext`, execution types |
| `carmy-schema` | JSON Schema generation |
| `carmy-macros` | `#[carmy::tool]` |
| `carmy-runtime` | registry, policies, validation, idempotency, cancellation, event stream |
| `carmy-http` | discovery, catalog, execution and SSE over Axum |
| `carmy-mcp` | the MCP server adapter over `rmcp` |
| `carmy-observability` | tracing subscriber setup |
| `carmy-cli` | `carmy new` |

`carmy-core` has no dependencies on HTTP, MCP, databases or LLM providers.

## The execution pipeline

`Runtime::execute(ExecutionRequest) -> ExecutionResult`:

1. Resolve the tool by name.
2. Run the policies: authorization, confirmation, quotas.
3. Validate the IDs, and validate the arguments against the input schema.
4. Reserve the idempotency identity: acquire, replay, conflict, or uncertain.
5. Invoke the tool under a deadline, the cancellation token and panic isolation.
6. Validate the output against the output schema.
7. Record the result.
8. Close the tracing span.

`Runtime::execute_stream` runs the same pipeline and yields events.

## Transports

A transport only translates:

1. its protocol into an `ExecutionRequest`, with the host's trusted `AgentContext`
2. its cancellation signal into the context's token
3. results and events into its own DTOs

Runtime types never go on the wire directly. Adding a transport requires no change to
tools or to the core.

## Conventions live in the facade

`State<T>`, auto-registration (collected at link time with `linkme`), `carmy.toml` and
`run()` are all in the `carmy` crate. The core and runtime stay free of them.

## Toward execution plans

A future plan (a DAG of tool calls with `$step.field` references) needs no new concepts:

- Each step is an `ExecutionRequest`.
- `parallel_safe` decides which steps can run concurrently.
- `effect` and `idempotent` decide what can be retried or cached.
- Per-step `request_id`s (`<plan>/<step>`) make resuming a failed plan safe.

## Review checklist

Check these for every change:

- Is the concept part of the domain, or part of a transport?
- Does HTTP or MCP leak into the runtime or the core?
- Does application state leak into `AgentContext`?
- Is a new abstraction needed now, or only allowed by the architecture?
- Does it prevent execution plans or a new transport later?
- Does it make the common case harder?
