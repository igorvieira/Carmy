---
title: Roadmap
description: "What's next for Carmy, and what's out of scope."
sidebar:
  order: 6
---

Carmy is at `0.2.0`, and its API is unstable during 0.x.

## Next

- **Execution plans:** DAGs of tool calls with `$step.field` references, on top of the
  existing runtime.
- **Tool progress events:** progress and partial results emitted by tools into the event
  stream.
- **Transports:** MCP Streamable HTTP, and approvals through MCP elicitation.
- **Idempotency stores:** durable adapters (Postgres, Redis) as separate crates.
- **CLI:** `carmy generate tool`.

## Out of scope

- a custom async runtime, HTTP parser or TLS stack
- an ORM, a distributed scheduler or a full workflow engine
- agent memory, a vector database, an LLM abstraction, a prompt framework or a model router

## Contributing

See [CONTRIBUTING.md](https://github.com/igorvieira/Carmy/blob/main/CONTRIBUTING.md). Use
conventional commits, and accompany behavior changes with contract tests.
