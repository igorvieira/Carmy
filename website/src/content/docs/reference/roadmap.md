---
title: Roadmap
description: "What's next for Carmy, and what's out of scope."
sidebar:
  order: 6
---

Carmy is at `0.4.0`, and its API is unstable during 0.x.

## Next

- **Execution plans:** DAGs of tool calls with `$step.field` references, on top of the
  existing runtime.
- **Tool progress events:** progress and partial results emitted by tools into the event
  stream.
- **Transports:** MCP Streamable HTTP, approvals through MCP elicitation, and MCP tasks
  on top of [jobs](/guides/jobs/).
- **Stores:** a Redis job and idempotency store next to the Postgres one.
- **Console:** the audit trail and the dead-letter queue in the terminal UI, not only
  over the protocol.
- **Sources of tools:** adapters that turn an existing GraphQL or OpenAPI API into Carmy
  tools, so its operations get effects, confirmation and replay-safe retries before an
  agent touches them. GraphQL is not a transport: agents call tools over MCP and HTTP.

## Out of scope

- a custom async runtime, HTTP parser or TLS stack
- an ORM, a distributed scheduler or a full workflow engine
- agent memory, a vector database, an LLM abstraction, a prompt framework or a model router

## Contributing

See [CONTRIBUTING.md](https://github.com/igorvieira/Carmy/blob/main/CONTRIBUTING.md). Use
conventional commits, and accompany behavior changes with contract tests.
