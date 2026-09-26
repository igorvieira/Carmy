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

Per-call logging is off everywhere. Apple M3 Pro, 2026-09-26. Two full runs agreed within
3% on latency and throughput; cold starts of a few milliseconds are noisy. Full method,
code and raw results:
[`benches/compare`](https://github.com/igorvieira/Carmy/tree/main/benches/compare).

### MCP over stdio

| server | cold start | p50 | p95 | throughput | p95 under load | peak memory |
|---|---|---|---|---|---|---|
| **Carmy** | 4 ms | 45 µs | 55 µs | 87,100 req/s | 501 µs | 11 MB |
| **Carmy**, `execution_meta(true)` | 4 ms | 49 µs | 58 µs | 66,200 req/s | 664 µs | 11 MB |
| rmcp, the Rust SDK, without Carmy | 3 ms | 52 µs | 62 µs | 40,800 req/s | 916 µs | 7 MB |
| TypeScript SDK | 135 ms | 52 µs | 61 µs | 72,600 req/s | 632 µs | 219 MB |
| Python SDK (`MCPServer`) | 351 ms | 456 µs | 489 µs | 3,100 req/s | 10,604 µs | 65 MB |

### HTTP

| server | cold start | p50 | p95 | throughput | p95 under load | peak memory |
|---|---|---|---|---|---|---|
| **Carmy** | 4 ms | 48 µs | 60 µs | 150,200 req/s | 727 µs | 13 MB |
| **Carmy** (1 thread) | 7 ms | 46 µs | 56 µs | 92,300 req/s | 733 µs | 13 MB |
| Axum, no guarantees | 4 ms | 44 µs | 55 µs | 158,500 req/s | 688 µs | 9 MB |
| Axum, no guarantees (1 thread) | 4 ms | 43 µs | 53 µs | 113,000 req/s | 621 µs | 10 MB |
| Express | 132 ms | 65 µs | 79 µs | 44,900 req/s | 1,589 µs | 115 MB |
| FastAPI (uvicorn) | 205 ms | 313 µs | 353 µs | 6,100 req/s | 13,210 µs | 50 MB |

### What the numbers say

- **MCP:** Carmy has the highest throughput and the lowest latency of these servers:
  - 87,100 req/s, against 72,600 for the TypeScript SDK.
  - A p50 of 45 µs, against 52 µs for both the TypeScript SDK and `rmcp`.
  - Its stdio uses readiness-driven pipes. `rmcp`'s default `stdio()` sends every read
    and write through tokio's blocking thread pool, which is why plain `rmcp` stops at
    40,800 req/s.
- **The cost of the guarantees:**
  - Against the same tool behind a plain Axum handler, Carmy adds about 3–4 µs at p50.
  - On all cores it costs 5% of throughput.
  - On one thread it costs 18%, down from 38% in 0.1.0. When the tool itself is nearly
    free, per-call validation, deadlines and policies show up.
- **`execution_meta(true)`** attaches `_meta` (execution ID and status) to every
  successful MCP result. It costs about 24% of throughput with a client that parses each
  extra object, such as `rmcp`'s. It is off by default. Errors always carry it.
- **Memory and cold start:**
  - Carmy uses 4–20× less memory than the Node and Python servers.
  - It starts in milliseconds, not hundreds of milliseconds.
  - It does use about 4 MB more than plain `rmcp` or Axum.
- **Against Python,** latency is 6–10× lower and throughput 25–28× higher.
- **Against Node,** throughput is 3.3× higher over HTTP and 1.2× higher over MCP, with
  lower latency on both.

### Changes since 0.1.0

Each change was found by profiling under this load, and each kept every guarantee:

| change | effect |
|---|---|
| MCP stdio over readiness-driven pipes, instead of the blocking thread pool | MCP throughput +58% |
| The idempotency fingerprint is computed only for requests with a `request_id` | HTTP (1 thread) +4.5% |
| Execution IDs no longer make a system call per request | HTTP (1 thread) +6% |
| Lifecycle events are built only when someone is streaming them | HTTP (1 thread) +10% |
| The deadline timer and cancellation waiter are registered only if the tool suspends | HTTP (1 thread) +6% |
| The HTTP adapter reads `Accept` without cloning every header | HTTP (1 thread) +2.7% |
| Execution metadata on successful MCP results became opt-in | MCP throughput +32% |

Overall, MCP throughput went from 38,900 to 87,100 req/s (+124%) and HTTP on one
thread from 69,600 to 92,300 req/s (+33%). `cargo bench -p carmy-benches` includes a
`guarantee/*` group with the cost of each guarantee on its own.

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
  pipe, with `rmcp`'s Rust client as the caller.
- **One machine.** Run `benches/compare/run.sh` on yours before relying on these numbers.
