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

## Comparisons

How does Carmy compare to the tool servers people write today? Every contender serves
**the same tool** (`search_products` over an in-memory catalog) to **the same client**. The
tool does almost no work, so the numbers show what each framework adds.

```console
benches/compare/run.sh        # needs Rust, Node + pnpm, and uv
```

Method: each contender runs once as a discarded priming run, then 3 measured runs; the
tables show the median. Each run measures:

- cold start
- a correctness check of the answer
- 5,000 sequential calls
- 10 seconds of load: 32 concurrent callers over MCP, 64 over HTTP
- peak memory

Apple M3 Pro, 2026-09-26. Two full runs agreed within 5% on latency, throughput and
memory. Cold starts of a few milliseconds are noisy. Full method, code and raw results:
[`benches/compare`](https://github.com/igorvieira/Carmy/tree/main/benches/compare).

### MCP over stdio

| server | cold start | p50 | p95 | throughput | p95 under load | peak memory |
|---|---|---|---|---|---|---|
| **Carmy** | 4 ms | 62 µs | 71 µs | 38,900 req/s | 966 µs | 10 MB |
| rmcp, the Rust SDK, without Carmy | 2 ms | 53 µs | 61 µs | 40,900 req/s | 886 µs | 7 MB |
| TypeScript SDK | 139 ms | 52 µs | 62 µs | 73,300 req/s | 610 µs | 221 MB |
| Python SDK (`MCPServer`) | 352 ms | 460 µs | 506 µs | 3,100 req/s | 10,774 µs | 65 MB |

### HTTP

| server | cold start | p50 | p95 | throughput | p95 under load | peak memory |
|---|---|---|---|---|---|---|
| **Carmy** | 6 ms | 52 µs | 62 µs | 143,200 req/s | 772 µs | 13 MB |
| **Carmy** (1 thread) | 7 ms | 50 µs | 61 µs | 69,600 req/s | 967 µs | 13 MB |
| Axum, no guarantees | 4 ms | 45 µs | 57 µs | 154,400 req/s | 713 µs | 9 MB |
| Axum, no guarantees (1 thread) | 4 ms | 43 µs | 55 µs | 113,000 req/s | 619 µs | 9 MB |
| Express | 135 ms | 64 µs | 77 µs | 44,300 req/s | 1,629 µs | 103 MB |
| FastAPI (uvicorn) | 205 ms | 310 µs | 352 µs | 6,100 req/s | 14,799 µs | 50 MB |

### What the numbers say

- **Carmy's guarantees are cheap per call.** Against the same tool behind a plain Axum
  handler, Carmy adds about 7 µs at p50 and costs 7% of throughput on all cores. On one
  thread the gap grows to 38%, because per-call work shows up when the tool itself is
  nearly free.
- **Memory and cold start.** Carmy uses 4–22× less memory than the Node and Python
  servers, and starts in milliseconds instead of hundreds of milliseconds. That matters
  when you run many agent servers, one per session or per tenant.
- **Against Python,** latency is 6–7× lower and throughput 12–23× higher.
- **Against Node,** the picture is mixed:
  - HTTP: Carmy has a lower p50 than Express and 3× its throughput.
  - MCP: the TypeScript SDK has **higher** throughput under concurrency, 73k against
    39k req/s.
  - Rust's `rmcp` without Carmy reaches about the same 41k, so the MCP ceiling here is
    the Rust SDK's stdio server loop, not Carmy.

### What the others don't do

The per-call cost buys behavior that the other servers don't provide by default:

| built in | Carmy | the others, as written here |
|---|---|---|
| a repeated `request_id` replays the result instead of running the side effect again | yes | no |
| declared effects, with destructive tools gated behind trusted confirmation | yes | no |
| errors with `recoverable`, `retryable`, `retry_after` and `suggested_action` | yes | no |
| a per-execution deadline and a cancellation token that reaches the tool | yes | no |
| input *and output* validated against the declared schemas | yes | varies |
| the same tool served over HTTP and MCP | yes | no |
| a tracing span per execution, without payloads | yes | no |

### Limits

- **This is a framework microbenchmark.** Real tools do I/O, which usually dwarfs these
  microseconds.
- **Node and Python serve on one thread,** while the Rust servers use every core by
  default. The `(1 thread)` rows compare like with like.
- **stdio is not a network.** MCP numbers measure JSON-RPC framing and dispatch over a
  pipe.
- **One machine.** Run `benches/compare/run.sh` on yours before relying on these numbers.
