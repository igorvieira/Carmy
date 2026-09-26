#!/usr/bin/env bash
# Runs every contender through the same driver and writes results/<date>.jsonl
# plus a markdown report. Requires: Rust (release build), Node + pnpm, uv.
set -euo pipefail
cd "$(dirname "$0")"
ROOT=../..
T=$ROOT/target/release
URL=http://127.0.0.1

cargo build --quiet --release -p carmy-compare --manifest-path $ROOT/Cargo.toml
(cd node && pnpm install --silent --frozen-lockfile)
(cd python && uv sync --quiet --frozen --python 3.12)

mkdir -p results
OUT=results/$(date -u +%Y-%m-%d).jsonl
: > "$OUT"
ARGS=(${COMPARE_ARGS:-})

mcp() { $T/driver mcp --name "$1" ${ARGS[@]+"${ARGS[@]}"} -- "${@:2}" | tee -a "$OUT"; }
http() {
  local name=$1 port=$2; shift 2
  $T/driver http --name "$name" --url "$URL:$port/agent/execute" --env "CARMY_ADDR=127.0.0.1:$port" \
    --env RUST_LOG=warn --concurrency 64 ${ARGS[@]+"${ARGS[@]}"} -- "$@" | tee -a "$OUT"
}
http1() {
  local name=$1 port=$2; shift 2
  $T/driver http --name "$name" --url "$URL:$port/agent/execute" --env "CARMY_ADDR=127.0.0.1:$port" \
    --env RUST_LOG=warn --env TOKIO_WORKER_THREADS=1 --concurrency 64 ${ARGS[@]+"${ARGS[@]}"} -- "$@" | tee -a "$OUT"
}

mcp "Carmy" $T/carmy_mcp mcp
mcp "rmcp (no Carmy)" $T/rmcp_mcp
mcp "TypeScript SDK" node node/mcp.mjs
mcp "Python SDK (MCPServer)" python/.venv/bin/python python/mcp_server.py

http "Carmy" 4201 $T/carmy_http
http1 "Carmy (1 thread)" 4202 $T/carmy_http
http "Axum, no guarantees" 4203 $T/axum_http
http1 "Axum, no guarantees (1 thread)" 4204 $T/axum_http
http "Express" 4205 node node/http.mjs
http "FastAPI (uvicorn)" 4206 python/.venv/bin/python python/http_server.py

$T/driver report "$OUT" | tee "${OUT%.jsonl}.md"
