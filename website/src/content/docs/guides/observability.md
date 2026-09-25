---
title: Observability
description: "Tracing spans for every execution, without leaking payloads."
sidebar:
  order: 11
---

Every execution runs in a `carmy.execution` tracing span with these fields:

| field | value |
|-------|-------|
| `execution_id` | the execution ID |
| `request_id` | the idempotency identity, when present |
| `tool` | the tool name |
| `effect` | the declared effect |
| `status` | `completed`, `failed`, `cancelled` or `timed_out` |
| `duration_ms` | the total duration |
| `replayed` | whether the result was replayed |
| `error_code` | the error code, on failure |

A completion event is logged at `INFO` on success and at `WARN` on failure. **Arguments
and outputs are never recorded**, because they may carry secrets.

```text
INFO carmy.execution{execution_id=exec_… request_id="abc" tool=create_order effect="write" status="completed" duration_ms=0 replayed=true}: execution finished
```

## Setup

`carmy::run()` installs a subscriber that writes to stderr and is filtered by
`RUST_LOG` (default `info`). Stdout stays free for MCP over stdio.

## OpenTelemetry

Carmy doesn't depend on OpenTelemetry. Compose the layers yourself, and the spans are
exported with the same fields:

```rust
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

tracing_subscriber::registry()
    .with(EnvFilter::new("info"))
    .with(carmy::observability::fmt_layer())
    .with(tracing_opentelemetry::layer().with_tracer(tracer))
    .init();
```

Install your subscriber before calling `run()`. If a global subscriber already exists,
Carmy leaves it in place.
