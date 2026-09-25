# Benchmarks

```console
cargo bench -p carmy-benches
```

The fixture tool (`benches/lib.rs`) does no work, so every measurement is Carmy's own
overhead on top of your tool.

| benchmark                           | measures |
|-------------------------------------|----------|
| `dispatch/direct_tool_call`         | baseline: calling the typed tool directly |
| `dispatch/runtime_execute`          | the full runtime pipeline: lookup, policy, schema validation, fingerprint, deadline, tracing span, output validation |
| `json/result_dto`                   | serializing an HTTP result DTO |
| `discovery/schema_generation`       | generating tool metadata and JSON Schemas |
| `discovery/tool_catalog_json`       | serializing the catalog (served pre-serialized over HTTP) |
| `http/execute_roundtrip`            | an in-process Axum request → runtime → response, without sockets |
| `concurrent/64_read_only_executions` | 64 spawned executions of a parallel-safe tool |

One local run, with short measurement times (Apple M3 Pro, rustc 1.93.0, commit at the
time of writing):

| benchmark                            | median   |
|--------------------------------------|----------|
| `dispatch/direct_tool_call`          | 90 ns    |
| `dispatch/runtime_execute`           | 2.5 µs   |
| `json/result_dto`                    | 0.31 µs  |
| `discovery/schema_generation`        | 2.1 µs   |
| `discovery/tool_catalog_json`        | 2.1 µs   |
| `http/execute_roundtrip`             | 4.7 µs   |
| `concurrent/64_read_only_executions` | 98 µs    |

These numbers describe one machine. Rerun the benchmarks before relying on them.
Comparisons with plain Axum or Actix handlers should measure Carmy's added overhead. They
should not suggest that Carmy replaces those frameworks.
