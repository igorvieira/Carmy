---
title: Error codes
description: "Error codes produced by Carmy itself."
sidebar:
  order: 4
---

Your tools define their own codes. These are the codes Carmy produces:

## Execution

| code | category | when |
|------|----------|------|
| `TOOL_NOT_FOUND` | `not_found` | no tool has this name |
| `INVALID_ARGUMENTS` | `validation` | the arguments don't match the input schema |
| `INVALID_ID` | `validation` | an empty execution ID, or a `request_id` longer than 256 bytes |
| `CONFIRMATION_REQUIRED` | `permission` | a destructive or confirmation-required tool without `confirm:<tool>` |
| `FORBIDDEN` | `permission` | `RequireToolPermission` is enabled and `tool:<name>` is missing |
| `IDEMPOTENCY_CONFLICT` | `conflict` | a `request_id` reused with a different tool or different arguments |
| `EXECUTION_UNCERTAIN` | `conflict` | a `request_id` whose execution is running or was interrupted |
| `IDEMPOTENCY_CAPACITY` | `capacity` | the in-memory idempotency store is full |
| `TIMEOUT` | `timeout` | the deadline passed; external effects may have committed |
| `CANCELLED` | `cancelled` | the execution was cancelled; external effects may have committed |
| `TOOL_PANIC` | `internal` | the tool panicked; external effects may have committed |
| `INVALID_OUTPUT` | `internal` | the output doesn't match the output schema, or can't be serialized |

## HTTP

| code | status | when |
|------|--------|------|
| `INVALID_REQUEST` | 400 | the body is not a valid execution request; see `details.reason` |
| `PAYLOAD_TOO_LARGE` | 413 | the body exceeds 1 MiB |

## Startup

These are returned by `build()`, `run()` and `listen()` as `carmy::Error::Registration`:

| code | when |
|------|------|
| `INVALID_TOOL_NAME` | a name is not 1–128 ASCII letters, digits, `_`, `-` or `.` |
| `DUPLICATE_TOOL` | two tools share a name |
| `INVALID_SCHEMA` | an input or output schema is not a valid JSON Schema |
| `MISSING_STATE` | a tool needs a `State<T>` that was never registered |
| `INVALID_SCHEDULE` | a `.schedule(..)` expression is not seven-field cron |
| `INVALID_WEBHOOK` | a `.webhook(..)` has no tool, an unknown tool, or `.enqueue()` without a queue |

## Jobs, webhooks and readiness

| code | category | when |
|------|----------|------|
| `JOBS_UNBOUND` | `internal` | `Jobs` was used before the app bound it to a runtime |
| `JOBS_CAPACITY` | `capacity` | the in-memory job store is full |
| `EXECUTION_UNCERTAIN` | `conflict` | a job's last attempt timed out, was cancelled or panicked on a non-idempotent tool; it is dead-lettered |
| `STORE_ERROR` | `internal` | a Postgres store could not read or write; retryable |
| `DATABASE_UNAVAILABLE` | `capacity` | `carmy::postgres::ready` found no answer from the pool |
| `WEBHOOK_UNAUTHORIZED` | `permission` | the delivery's signature or token is missing, expired or wrong (HTTP 401) |
| `READY_TIMEOUT` | `timeout` | a readiness check took longer than 5 seconds |
| `WORKER_DOWN` | `capacity` | `.require_worker(within)` saw no worker tick in time |
| `UNAVAILABLE` | `not_found` | the console's `audit` or `dead` with no trail or queue attached |
