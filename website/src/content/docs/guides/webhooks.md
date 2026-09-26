---
title: Webhooks
description: "Receive signed deliveries from any sender as idempotent tool executions."
sidebar:
  order: 14
---

A webhook is a tool with a sender at the other end. Carmy verifies the delivery on the
raw body, reads its identity from the payload as the `request_id`, and runs the tool, so
a redelivery **replays** instead of running twice.

```rust
use carmy::http::Webhook;

carmy::app()
    .webhook("/webhooks/billing", Webhook::hmac_sha256(secret, "X-Signature")
        .tool("billing_event")
        .event_id("/id")
        .enqueue())
    .webhook("/webhooks/chat", Webhook::shared_secret("X-Token", token)
        .tool("chat_update")
        .event_id("/update_id"))
```

Carmy knows no provider. It offers three generic checks, and the app states the
header, the secret and where the identity lives:

| constructor | verifies |
|-------------|----------|
| `Webhook::hmac_sha256(secret, header)` | `header` holds the hex HMAC-SHA256 of the body, with or without a `sha256=` prefix |
| `Webhook::shared_secret(header, secret)` | `header` equals a secret shared with the sender |
| `Webhook::custom(\|headers, body\| ..)` | any other scheme: timestamped signatures, other algorithms, allow-lists |

Secrets are compared in constant time. Without a valid delivery the tool never sees the
body: the answer is `401` with `WEBHOOK_UNAUTHORIZED`. A body that is not JSON answers
`400` with `INVALID_REQUEST`.

## Identity

`.event_id("/id")` is a [JSON pointer](https://datatracker.ietf.org/doc/html/rfc6901)
into the payload: `/id`, `/event/id`, `/update_id`. Strings and numbers become the
`request_id`. Without it, or when the pointer finds nothing, deliveries are not
deduplicated and every one runs.

## Inline or enqueued

Without `.enqueue()` the tool runs inline and the response is the usual
[`ResultDto`](/transports/http/) with the execution's status. With it, the delivery is
accepted at once with `202` and `{ "job_id", "request_id" }`, and a worker runs the tool
as a [job](/guides/jobs/). **Enqueue is the recommended mode:** senders time out in
seconds and retry, and a job survives a restart and retries on its own.

## A custom scheme

A sender that signs `"{timestamp}.{body}"` and rejects old timestamps fits in
`Webhook::custom`:

```rust
Webhook::custom(move |headers, body| {
    let Some((t, signature)) = parse_signature_header(headers) else { return false };
    fresh(t, Duration::from_secs(300)) && hmac_matches(&secret, &[t.as_bytes(), b".", body], signature)
})
```

## The tool

The verified payload is the tool's input. Declare only what you read:

```rust
#[derive(Deserialize, JsonSchema)]
struct BillingEvent {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    data: serde_json::Value,
}

#[carmy::tool(description = "Apply a subscription event", effect = "write")]
async fn billing_event(State(store): State<Arc<Store>>, input: BillingEvent) -> AgentResult<String> { .. }
```

The tool is an ordinary tool: agents can call it, the console can call it, and its
effect, schema and audit record are the same. A misconfigured webhook (no tool, an
unknown tool, `.enqueue()` without a queue) fails when the app builds, not on the first
delivery.

## Own routes

`Webhook::verify(&headers, &body)` is public, for hosts that mount webhooks on their own
routes and want the same verification.
