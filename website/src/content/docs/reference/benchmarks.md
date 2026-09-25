---
title: Benchmarks
description: "How Carmy's overhead is measured."
sidebar:
  order: 5
---

```console
cargo bench -p carmy-benches
```

The fixture tool does no work, so every number is Carmy's own overhead on top of your
code.

| benchmark | measures |
|-----------|----------|
| `dispatch/direct_tool_call` | baseline: calling the typed tool directly |
| `dispatch/runtime_execute` | the full runtime pipeline |
| `json/result_dto` | serializing an HTTP result |
| `discovery/schema_generation` | generating metadata and schemas |
| `discovery/tool_catalog_json` | serializing the catalog |
| `http/execute_roundtrip` | an in-process Axum request, runtime call and response |
| `concurrent/64_read_only_executions` | 64 concurrent executions |

One local run (Apple M3 Pro, rustc 1.93.0):

| benchmark | median |
|-----------|--------|
| `dispatch/direct_tool_call` | 90 ns |
| `dispatch/runtime_execute` | 2.5 µs |
| `json/result_dto` | 0.31 µs |
| `discovery/schema_generation` | 2.1 µs |
| `discovery/tool_catalog_json` | 2.1 µs |
| `http/execute_roundtrip` | 4.7 µs |
| `concurrent/64_read_only_executions` | 98 µs |

These numbers describe one machine; rerun the benchmarks before relying on them. Carmy
makes no performance claims that these benchmarks cannot reproduce.
