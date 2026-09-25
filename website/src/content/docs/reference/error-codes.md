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
