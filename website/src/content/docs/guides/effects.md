---
title: Effects and confirmation
description: "Declare side effects explicitly, and gate destructive tools behind trusted confirmation."
sidebar:
  order: 3
---

Every tool declares its effect. Carmy never infers it from names or HTTP methods.

| effect | meaning | example |
|--------|---------|---------|
| `none` | pure computation | formatting, math |
| `read` | reads state, changes nothing | search, lookup |
| `write` | changes your system | create an order |
| `external_write` | changes a system you don't control | send an email, charge a card |
| `destructive` | irreversible or data-destroying | delete a customer |

Effects are published in discovery (`GET /agent/tools`) and in the MCP tool annotations,
so clients and hosts can apply approval policies *before* calling a tool.

## Confirmation

```rust
#[carmy::tool(
    description = "Delete a customer permanently",
    effect = "destructive",
    confirmation = "required"
)]
async fn delete_customer(input: CustomerId) -> AgentResult<()> { /* … */ Ok(()) }
```

The default policy, `SafePolicy`, rejects destructive tools and tools declaring
`confirmation = "required"` unless the trusted context grants `confirm:<tool>`:

```json
{ "error": { "code": "CONFIRMATION_REQUIRED", "category": "permission", "recoverable": false, "retryable": false, "message": "Trusted confirmation is required" } }
```

The grant comes from the **host**, never from tool arguments or request bodies. Typically,
your UI asks a human, and your authentication middleware inserts the permission:

```rust
ctx.permissions.insert("confirm:delete_customer".into());
```

See [Security](/guides/security/) for where the host sets the context.

## Parallel safety and idempotency

- `parallel_safe = false` (the default) serializes executions of that tool. Set it to
  `true` when concurrent calls are safe.
- `idempotent = true` declares that repeating the call is harmless. Together with a `read`
  or `none` effect, it lets Carmy mark results `cacheable` for clients.
