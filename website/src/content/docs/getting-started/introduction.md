---
title: Introduction
description: "What Carmy is, and why agents need a deterministic execution layer."
sidebar:
  order: 1
---

Carmy is **agent-native execution infrastructure for Rust**. It is not another
general-purpose HTTP framework: it does not route web pages and does not compete with
Axum, Actix or Hyper. It builds on them.

> LLM reasoning can be probabilistic. Side effects should not be.

## Why agents need it

Agents call tools in ways that ordinary web clients do not:

- **They retry blindly.** After a timeout, a network failure or a model retry, an agent
  calls again, often without knowing whether the first attempt committed.
- **They read machines, not docs.** Agents discover tools from schemas and metadata, and
  they react to error codes rather than prose.
- **They give up midway.** Executions get abandoned, and background work must not keep
  running with undefined semantics.
- **They need guard rails.** A tool that deletes data must be identifiable, and gated,
  *before* it runs.

Carmy makes each of these explicit in the runtime, so every tool gets the same
guarantees.

## Core concepts

| concept | what it is |
|---------|------------|
| **Tool** | A typed Rust function with metadata: name, description, schemas, effect, idempotency, parallel safety and confirmation. |
| **Execution** | One run of a tool. It has an `execution_id`, an optional `request_id` for retries, a status and a result. |
| **Effect** | The declared side effect of a tool: `none`, `read`, `write`, `external_write` or `destructive`. |
| **Context** | `AgentContext`: trusted framework data (principal, session, permissions, cancellation). It holds no application state. |
| **Policy** | A hook that runs before every execution: authorization, confirmation, quotas. |
| **Transport** | An adapter that turns a protocol (HTTP, MCP) into runtime executions. |

## The invariants

- The core does not know about transports, and tools do not know about HTTP.
- The runtime does not know about LLM providers.
- Effects are explicit, and errors are machine-readable.
- Retries are safe, and execution is observable.
- Transports are replaceable.

## What Carmy is not

Carmy has no custom async runtime, HTTP parser, TLS, ORM, workflow engine, agent memory,
vector database, LLM abstraction or prompt framework. Those are separate problems.

Next: [Quick start](/getting-started/quick-start/).
