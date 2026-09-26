---
title: Console
description: "Explore and call tools from a terminal UI, or drive them as an agent over JSON Lines."
sidebar:
  order: 12
---

The console runs your application's tools through the real runtime, with its policies,
validation, idempotency, deadlines and tracing. It has two faces:

- **For people:** `carmy console` opens a terminal UI.
- **For agents and scripts:** the application speaks `carmy-console/1`, a JSON Lines
  protocol, on stdio.

## Terminal UI

```console
carmy console
```

![The Carmy console showing a replayed call](/console/tui.png)

It builds your application, then shows the following:

| area | what it shows |
|------|---------------|
| **tools** | every tool with an effect badge: `N` none, `R` read, `W` write, `X` external write, `D` destructive; 🔒 until confirmation is granted |
| **details** | the description, the flags, and the input and output fields |
| **arguments** | JSON, prefilled from the input schema |
| **request_id** | optional; a repeated call with the same ID shows `replayed` |
| **result** | the status, the duration and the data, or the structured error with its `suggested_action` |

| key | action |
|-----|--------|
| `↑` `↓` / `j` `k` | select a tool, or scroll the result |
| `Tab` / `Shift+Tab` | move focus |
| `Enter` | run the selected tool |
| `Ctrl+R` | reset the arguments to the schema's example |
| `Ctrl+U` | clear the field being edited |
| `c` | grant or revoke `confirm:<tool>` for this session |
| `r` | run the last call again |
| `h` | history; `Enter` runs a past call again |
| `?` | help |
| `q`, `Esc`, `Ctrl+C` | quit |

Arguments and `request_id`s are kept per tool, because reusing a `request_id` with
another tool is an idempotency conflict.

### Destructive tools

A tool marked 🔒 won't run until it's confirmed. Select it and press `c`:

![Confirmation modal for a destructive tool](/console/confirm.png)

After `y`, the lock turns into ✓ for this session and the call goes through:

![The destructive tool confirmed and run](/console/confirmed.png)

The grant is the same `confirm:<tool>` permission a host sets in production. Press `c`
again to revoke it.

### History

`h` lists every call of the session. `Enter` runs one again, with the same arguments
and `request_id`:

![The history of the session](/console/history.png)

## For agents: `carmy-console/1`

```console
cargo run -- console          # or: carmy console --jsonl
```

`carmy console` also switches to this mode when its stdin or stdout isn't a terminal.
Each message is one JSON object per line. The first line is:

```json
{"event":"ready","protocol":"carmy-console/1","server":"shop","tools":3}
```

| request | result |
|---------|--------|
| `{"id":1,"op":"tools"}` | the catalog, in the same shape as `GET /agent/tools` |
| `{"id":2,"op":"describe","tool":"create_order"}` | the tool's metadata and schemas |
| `{"id":3,"op":"call","tool":"create_order","arguments":{"sku":"KB-01"},"request_id":"r1"}` | the same body as `POST /agent/execute`, plus `replayed` and `duration_ms` |
| `{"id":4,"op":"confirm","tool":"cancel_order"}` | grants `confirm:cancel_order` for this session |
| `{"id":5,"op":"revoke","tool":"cancel_order"}` | withdraws the grant |
| `{"op":"help"}` / `{"op":"exit"}` | lists the operations / ends the session |

Every response echoes `id` and carries `ok`:

```json
{"id":3,"ok":true,"result":{"execution_id":"exec_…","status":"completed","data":{"order_id":1},"_agent":{"cacheable":false,"next_actions":[]},"replayed":false,"duration_ms":0.4}}
{"id":9,"ok":false,"error":{"code":"TOOL_NOT_FOUND","message":"unknown tool `x`","category":"not_found","recoverable":true,"retryable":false,"suggested_action":"tools"}}
```

- **A failed execution is still `ok: true`.** The execution's own error is in
  `result.error`, exactly as over HTTP. `ok: false` means the request itself was bad:
  `INVALID_REQUEST`, `UNKNOWN_OP` or `TOOL_NOT_FOUND`.
- **Text commands:** a line that doesn't start with `{` is read as one, which is handy
  in a terminal: `tools`, `describe <tool>`, `confirm <tool>`, `revoke <tool>`, `help`,
  `exit`, or `<tool> [json] [--request-id <id>]`. The answers are still JSON.
- **The session:** it runs as principal `console`, with its own session, so
  `request_id` replays work across calls. Whoever runs the process is the trusted
  operator, so `confirm` grants apply only to this session.
- **Logs:** they go to stderr, at `warn` by default. Stdout carries only the protocol.

## Embedding

`carmy::console::serve(runtime, name, input, output)` serves the protocol over any
async reader and writer, which is useful in tests.
