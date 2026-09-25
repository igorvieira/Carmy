---
title: Erros
description: "Erros legíveis por máquina, com códigos, categorias e dicas de recuperação."
sidebar:
  order: 4
---

Os erros são pensados primeiro para máquinas:

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

| campo | significado |
|-------|-------------|
| `code` | um identificador estável para tomar decisões |
| `message` | uma explicação legível por humanos |
| `category` | uma classe ampla; veja abaixo |
| `recoverable` | o agente pode corrigir a requisição (mudar os argumentos) e seguir |
| `retryable` | repetir a mesma requisição pode dar certo |
| `retry_after` | segundos de espera antes de repetir, quando se sabe |
| `suggested_action` | uma tool que o agente deveria considerar em seguida |
| `details` | dados extras estruturados |

## Retornando erros

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

| builder | efeito |
|---------|--------|
| `.recoverable()` | define `recoverable` |
| `.retryable(Some(5))` | define `retryable`, `recoverable` e `retry_after` |
| `.suggest("tool")` | define `suggested_action` |
| `.details(json!({…}))` | define `details` |

## Categorias

`validation`, `not_found`, `permission`, `conflict`, `cancelled`, `timeout`, `internal`,
`capacity`.

O adaptador HTTP mapeia categorias para status codes (veja [HTTP](/pt-br/transports/http/)),
mas o corpo é o que vale. Os códigos que o próprio runtime produz estão em
[Códigos de erro](/pt-br/reference/error-codes/).
