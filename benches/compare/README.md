# Comparison benchmarks

Carmy against the tool servers people write today, all serving **the same tool**
(`search_products` over a fixed in-memory catalog) to **the same client** (`driver`).

```console
benches/compare/run.sh        # needs Rust, Node + pnpm, and uv
```

Results land in `results/<date>.jsonl` and `results/<date>.md`.

## Contenders

| transport | contender | code |
|-----------|-----------|------|
| MCP (stdio) | Carmy | `src/bin/carmy_mcp.rs` |
| MCP (stdio) | Carmy with `execution_meta(true)` (`_meta` on every result) | `src/bin/carmy_mcp_meta.rs` |
| MCP (stdio) | rmcp, the official Rust SDK, without Carmy | `src/bin/rmcp_mcp.rs` |
| MCP (stdio) | the official TypeScript SDK (`McpServer`) | `node/mcp.mjs` |
| MCP (stdio) | the official Python SDK (`MCPServer`, formerly FastMCP) | `python/mcp_server.py` |
| HTTP | Carmy (`POST /agent/execute`) | `src/bin/carmy_http.rs` |
| HTTP | a plain Axum handler with none of Carmy's guarantees | `src/bin/axum_http.rs` |
| HTTP | Express | `node/http.mjs` |
| HTTP | FastAPI on uvicorn | `python/http_server.py` |

Versions are pinned by `Cargo.lock`, `node/pnpm-lock.yaml` and `python/uv.lock`.

## Method

For each contender, `driver` spawns a fresh server for every run. It does one
discarded **priming run** (macOS scans a freshly built binary on first launch), then 3
measured runs, and reports the **median** of each metric across them.

Every run:

1. **Cold start:** measures the time from spawn to the first answer (the MCP
   `initialize` handshake, or the first HTTP `200`).
2. **Correctness check:** confirms the answer. The tool must return the 4 products
   matching `mouse`, so a server can't win by failing fast.
3. **Warm-up:** makes 500 calls that are not measured.
4. **Sequential latency:** makes 5,000 calls, one at a time, and records p50, p95 and
   p99.
5. **Concurrency:** runs 32 concurrent callers (MCP, over one stdio connection) or 64
   (HTTP, keep-alive) for 10 seconds, recording throughput and p50/p95/p99.
6. **Peak RSS:** samples the server process every 25 ms.

Per-request logging is off everywhere (`RUST_LOG=warn`, uvicorn `access_log=False`,
the MCP servers' default or `WARNING` level).

## Reading the results

- **This is a framework microbenchmark.** Real tools do I/O, which dominates in
  production. These numbers bound what the framework adds on top of that.
- **Carmy does more work per call than the plain Axum handler.** It validates the input
  and output schemas, runs policies, and handles deadlines, cancellation and tracing.
  The "Axum, no guarantees" row shows what those guarantees cost.
- **Node and Python serve on one thread.** The Rust servers use every core by default,
  so the `(1 thread)` rows compare like with like.
- **stdio is not a network.** MCP numbers measure JSON-RPC framing and dispatch over a
  pipe.
