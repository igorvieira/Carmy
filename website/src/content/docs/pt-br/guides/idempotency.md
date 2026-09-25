---
title: Idempotência
description: "Torne seguros os retries dos agentes com request IDs e resultados registrados."
sidebar:
  order: 5
---

Agentes repetem chamadas depois de timeouts, falhas de rede e retries do modelo, muitas
vezes sem saber se a primeira tentativa foi efetivada. O Carmy torna esses retries seguros.

## Como funciona

Envie um `request_id` em qualquer chamada que você possa repetir:

```json
{ "tool": "create_order", "arguments": { "sku": "KB-01" }, "request_id": "order-7f3" }
```

O runtime reserva a identidade de forma atômica **antes** de a tool rodar:

- **A identidade** é o principal, a sessão e o `request_id`.
- **O fingerprint** é um hash do nome da tool, dos argumentos e dos metadados da
  requisição.

| situação | resultado |
|----------|-----------|
| identidade nova | a tool roda e o resultado é registrado |
| mesma identidade e fingerprint, já concluída | o resultado registrado é **reenviado**; a tool não roda de novo |
| mesma identidade, fingerprint diferente | `IDEMPOTENCY_CONFLICT` (HTTP 409) |
| mesma identidade, ainda rodando ou interrompida | `EXECUTION_UNCERTAIN` (HTTP 409) |

```console
$ curl … -d '{"tool":"create_order","arguments":{"sku":"KB-01"},"request_id":"abc"}'
{"execution_id":"exec_d8df…","status":"completed","data":{"order_id":1},…}

$ curl … -d '{"tool":"create_order","arguments":{"sku":"KB-01"},"request_id":"abc"}'
{"execution_id":"exec_d8df…","status":"completed","data":{"order_id":1},…}   # reenviado, mesmo execution_id
```

## A incerteza é explícita

Timeouts e cancelamentos são registrados como qualquer outro resultado, porque o efeito
externo pode já ter sido efetivado. Repetir uma requisição que deu timeout devolve o
timeout; a escrita não roda de novo às escondidas.

Uma execução interrompida no meio (o processo morreu, o cliente desconectou) mantém a
reserva e responde `EXECUTION_UNCERTAIN`. O agente deve reconciliar (por exemplo, buscando
o pedido) em vez de repetir às cegas.

## Stores

O `InMemoryIdempotencyStore` padrão é local ao processo e limitado (10.000 entradas).
Quando fica cheio, novas requisições falham de forma segura com `IDEMPOTENCY_CAPACITY`.
Ele nunca descarta entradas que podem representar efeitos já efetivados.

Para deploys duráveis ou com várias instâncias, implemente `IdempotencyStore`:

```rust
use carmy::runtime::{IdempotencyKey, IdempotencyStore, Reservation, StoreFuture};

struct PostgresStore { /* … */ }

impl IdempotencyStore for PostgresStore {
    /// Compara o fingerprint e reserva uma identidade livre, de forma atômica.
    fn reserve<'a>(&'a self, key: &'a IdempotencyKey, fingerprint: &'a str)
        -> StoreFuture<'a, Reservation> { todo!() }
    /// Registra o resultado de forma durável antes de retornar.
    fn complete<'a>(&'a self, key: &'a IdempotencyKey, fingerprint: &'a str,
        result: &'a carmy::ExecutionResult) -> StoreFuture<'a, ()> { todo!() }
}

carmy::app().idempotency_store(Arc::new(PostgresStore { /* … */ })).run().await
```
