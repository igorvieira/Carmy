---
title: Idempotency
description: "Make agent retries safe with request IDs and recorded results."
sidebar:
  order: 5
---

Agents retry after timeouts, network failures and model retries, often without knowing
whether the first attempt committed. Carmy makes retries safe.

## How it works

Send a `request_id` with any call you might retry:

```json
{ "tool": "create_order", "arguments": { "sku": "KB-01" }, "request_id": "order-7f3" }
```

The runtime reserves the identity atomically **before** the tool runs:

- **The identity** is the principal, the session and the `request_id`.
- **The fingerprint** is a hash of the tool name, the arguments and the request
  metadata.

| situation | outcome |
|-----------|---------|
| new identity | the tool runs and the result is recorded |
| same identity and fingerprint, finished | the recorded result is **replayed**; the tool does not run again |
| same identity, different fingerprint | `IDEMPOTENCY_CONFLICT` (HTTP 409) |
| same identity, still running or interrupted | `EXECUTION_UNCERTAIN` (HTTP 409) |

```console
$ curl … -d '{"tool":"create_order","arguments":{"sku":"KB-01"},"request_id":"abc"}'
{"execution_id":"exec_d8df…","status":"completed","data":{"order_id":1},…}

$ curl … -d '{"tool":"create_order","arguments":{"sku":"KB-01"},"request_id":"abc"}'
{"execution_id":"exec_d8df…","status":"completed","data":{"order_id":1},…}   # replayed, same execution_id
```

## Uncertainty is explicit

Timeouts and cancellations are recorded like any other result, because the external
effect may already have committed. Retrying a timed-out request replays the timeout; it
does not silently run the write again.

An execution that was dropped mid-flight (the process died, the client disconnected)
keeps its reservation and reports `EXECUTION_UNCERTAIN`. The agent should reconcile
(for example, by searching for the order) instead of blindly retrying.

## Stores

The default `InMemoryIdempotencyStore` is process-local and bounded (10,000 entries). When
it is full, new requests fail closed with `IDEMPOTENCY_CAPACITY`. It never evicts entries
that may represent committed effects.

For durable or multi-instance deployments, implement `IdempotencyStore`:

```rust
use carmy::runtime::{IdempotencyKey, IdempotencyStore, Reservation, StoreFuture};

struct PostgresStore { /* … */ }

impl IdempotencyStore for PostgresStore {
    /// Atomically compare the fingerprint and reserve a vacant identity.
    fn reserve<'a>(&'a self, key: &'a IdempotencyKey, fingerprint: &'a str)
        -> StoreFuture<'a, Reservation> { todo!() }
    /// Durably record the result before returning.
    fn complete<'a>(&'a self, key: &'a IdempotencyKey, fingerprint: &'a str,
        result: &'a carmy::ExecutionResult) -> StoreFuture<'a, ()> { todo!() }
}

carmy::app().idempotency_store(Arc::new(PostgresStore { /* … */ })).run().await
```
