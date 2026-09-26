
### MCP over stdio

| server | cold start | p50 | p95 | p99 | throughput | p95 under load | peak RSS |
|---|---|---|---|---|---|---|---|
| Carmy | 4 ms | 45 µs | 55 µs | 61 µs | 87143 req/s | 501 µs | 11.2 MB |
| Carmy (execution_meta on) | 4 ms | 49 µs | 58 µs | 63 µs | 66246 req/s | 664 µs | 11.1 MB |
| rmcp (no Carmy) | 3 ms | 52 µs | 62 µs | 76 µs | 40805 req/s | 916 µs | 7.1 MB |
| TypeScript SDK | 135 ms | 52 µs | 61 µs | 73 µs | 72609 req/s | 632 µs | 219.3 MB |
| Python SDK (MCPServer) | 351 ms | 456 µs | 489 µs | 555 µs | 3090 req/s | 10604 µs | 65.0 MB |

### HTTP

| server | cold start | p50 | p95 | p99 | throughput | p95 under load | peak RSS |
|---|---|---|---|---|---|---|---|
| Carmy | 4 ms | 48 µs | 60 µs | 72 µs | 150205 req/s | 727 µs | 13.0 MB |
| Carmy (1 thread) | 7 ms | 46 µs | 56 µs | 65 µs | 92337 req/s | 733 µs | 12.9 MB |
| Axum, no guarantees | 4 ms | 44 µs | 55 µs | 66 µs | 158461 req/s | 688 µs | 9.2 MB |
| Axum, no guarantees (1 thread) | 4 ms | 43 µs | 53 µs | 60 µs | 113030 req/s | 621 µs | 9.5 MB |
| Express | 132 ms | 65 µs | 79 µs | 100 µs | 44897 req/s | 1589 µs | 115.4 MB |
| FastAPI (uvicorn) | 205 ms | 313 µs | 353 µs | 412 µs | 6075 req/s | 13210 µs | 49.8 MB |
