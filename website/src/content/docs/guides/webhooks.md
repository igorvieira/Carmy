---
title: Webhooks
description: "A door into a tool: verified deliveries from any sender, run as idempotent executions."
sidebar:
  order: 14
---

A webhook is a door into a tool. Carmy checks the delivery, reads its identity from the
payload as the `request_id`, and runs the tool, so a redelivery **replays** instead of
running twice. Carmy knows no sender: you say how to check a delivery.

```rust
use carmy::http::{Webhook, verify};

carmy::app()
    .webhook("/webhooks/billing", Webhook::to("billing_event")
        .verify(verify::hmac_sha256(secret, "X-Signature"))
        .event_id("/id")
        .enqueue())
```

| method | does |
|--------|------|
| `Webhook::to(tool)` | the tool that receives the verified payload as its input |
| `.verify(check)` | how to tell an authentic delivery; required |
| `.event_id("/id")` | a [JSON pointer](https://datatracker.ietf.org/doc/html/rfc6901) to the delivery's identity; strings and numbers become the `request_id` |
| `.enqueue()` | answer `202` at once and run the tool as a [job](/guides/jobs/) |
| `.unverified()` | accept every delivery, for senders authenticated upstream (a gateway, a private network) |

## Verifying

`.verify` takes any function from a `Delivery` (the headers and the raw body) to
`bool`. Two common checks come ready:

| check | accepts when |
|-------|--------------|
| `verify::hmac_sha256(secret, header)` | `header` holds the hex HMAC-SHA256 of the body, with or without `sha256=` |
| `verify::shared_secret(header, secret)` | `header` equals a secret shared with the sender |

Anything else is a closure. Compare secrets in constant time:

```rust
Webhook::to("partner_event").verify(move |delivery| {
    let Some(signature) = delivery.header("x-partner-signature") else { return false };
    my_scheme::is_valid(&secret, signature, delivery.body)
})
```

A delivery that fails the check never reaches the tool: `401` with
`WEBHOOK_UNAUTHORIZED`. A body that is not JSON answers `400` with `INVALID_REQUEST`.

## Safe by default

The app refuses to start, with `INVALID_WEBHOOK` and the reason, when a webhook:

- has no `.verify(..)` and no `.unverified()`;
- targets a tool that does not exist;
- uses `.enqueue()` without a job queue;
- takes a path already in use, or one that does not start with `/`.

Without `.event_id(..)`, or when the pointer finds nothing, deliveries are not
deduplicated and every one runs.

## Inline or enqueued

Without `.enqueue()` the tool runs inline and the response is the usual
[`ResultDto`](/transports/http/). With it, the answer is `202` with
`{ "job_id", "request_id" }`, and a worker runs the tool. **Enqueue is the recommended
mode:** senders time out in seconds and retry, and a job survives a restart and retries
on its own.

## For agents

Webhooks are listed in discovery and in the console (`webhooks`), never with their
secrets:

```json
"webhooks": [
  { "path": "/webhooks/billing", "tool": "billing_event", "mode": "enqueue", "event_id": "/id", "verified": true }
]
```

An agent that needs to replay or simulate a delivery calls the tool directly with the
payload and the same `request_id`. It is the same path, with the same effect, replay and
audit, and no signature to forge.

## Own routes

`webhook.check(&headers, &body)` is public, for hosts that mount webhooks on their own
routes and want the same verification.
