---
title: Errors
description: "Machine-readable errors with codes, categories and recovery hints."
sidebar:
  order: 4
---

Errors are designed for machines first:

```json
{
  "error": {
    "code": "PRODUCT_NOT_FOUND",
    "message": "No product has this SKU",
    "category": "not_found",
    "recoverable": true,
    "retryable": false,
    "suggested_action": "search_products"
  }
}
```

| field | meaning |
|-------|---------|
| `code` | a stable identifier to branch on |
| `message` | a human-readable explanation |
| `category` | a coarse class; see below |
| `recoverable` | the agent can fix the request (change arguments) and continue |
| `retryable` | retrying the same request may succeed |
| `retry_after` | seconds to wait before retrying, if known |
| `suggested_action` | a tool the agent should consider next |
| `details` | structured extra data |

## Returning errors

```rust
use carmy::prelude::*;

#[carmy::tool(effect = "write")]
async fn create_order(input: CreateOrder) -> AgentResult<Order> {
    if !catalog_has(&input.sku) {
        return Err(AgentError::new(
            "PRODUCT_NOT_FOUND",
            "No product has this SKU",
            ErrorCategory::NotFound,
        )
        .recoverable()
        .suggest("search_products"));
    }
    // …
}
```

| builder | effect |
|---------|--------|
| `.recoverable()` | sets `recoverable` |
| `.retryable(Some(5))` | sets `retryable`, `recoverable` and `retry_after` |
| `.suggest("tool")` | sets `suggested_action` |
| `.details(json!({…}))` | sets `details` |

## Categories

`validation`, `not_found`, `permission`, `conflict`, `cancelled`, `timeout`, `internal`,
`capacity`.

The HTTP adapter maps categories to status codes (see [HTTP](/transports/http/)), but the
body is authoritative. The codes the runtime produces itself are listed in
[Error codes](/reference/error-codes/).
