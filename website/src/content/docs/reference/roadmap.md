---
title: Roadmap
description: "What's next for Carmy, and what's out of scope."
sidebar:
  order: 6
---

Carmy is at `0.6.1`, and its API is unstable during 0.x.

## Next

- **GraphQL as a source of tools:** operations from a GraphQL schema and an operations
  file become tools, the way [OpenAPI](/guides/openapi/) operations do. GraphQL is not a
  transport: agents call tools over MCP and HTTP.
- **OpenAPI in YAML**, next to JSON.
- **Next actions from tools:** tools suggesting their own follow-ups, next to the ones
  Carmy already knows.
- **Durable MCP tasks:** MCP tasks backed by [jobs](/guides/jobs/), so they survive a
  restart.
- **Audit on Redis**, next to Postgres.

Execution plans (DAGs of tool calls) are no longer planned: composition stays with the
agent, or with tools that enqueue other tools.

## Out of scope

- a custom async runtime, HTTP parser or TLS stack
- an ORM
- a workflow engine: no DAGs of steps, sagas or state kept between steps. Carmy's
  [jobs](/guides/jobs/) run **one tool call later**, with retries and schedules, and
  that is where they stop
- a distributed scheduler beyond that: no leader election or cluster membership; the
  database's row locks are the only coordination
- agent memory, a vector database, an LLM abstraction, a prompt framework or a model router

## Contributing

See [CONTRIBUTING.md](https://github.com/igorvieira/Carmy/blob/main/CONTRIBUTING.md). Use
conventional commits, and accompany behavior changes with contract tests.
