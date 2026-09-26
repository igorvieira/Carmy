---
title: Webhooks
description: "Receive Stripe, Telegram or any signed delivery as an idempotent tool execution."
sidebar:
  order: 14
---

A webhook is a tool with a provider at the other end. Carmy verifies the delivery on
the raw body, uses the provider's event id as the `request_id`, and runs the tool, so a
redelivery **replays** instead of running twice.

```rust
use carmy::http::Webhook;

carmy::app()
    .webhook("/webhooks/stripe", Webhook::stripe(secret).tool("membership_event").enqueue())
    .webhook("/webhooks/telegram", Webhook::telegram(token).tool("telegram_update"))
    .webhook("/webhooks/github", Webhook::hmac_sha256(secret, "X-Hub-Signature-256")
        .tool("github_event")
        .event_id(|body| body["delivery"].as_str().map(str::to_owned)))
```

| constructor | verifies | `request_id` |
|-------------|----------|--------------|
| `Webhook::stripe(secret)` | `Stripe-Signature` (`t=`, `v1=`) over `"{t}.{body}"`, within 5 minutes (`.tolerance(..)`) | the event `id` |
| `Webhook::telegram(token)` | `X-Telegram-Bot-Api-Secret-Token` | `update_id` |
| `Webhook::hmac_sha256(secret, header)` | hex HMAC-SHA256 of the body in `header`, with or without `sha256=` | none until `.event_id(..)` |

Signatures are compared in constant time. Without a valid one the tool never sees the
body: the answer is `401` with `WEBHOOK_UNAUTHORIZED`. A body that is not JSON answers
`400` with `INVALID_REQUEST`.

## Inline or enqueued

Without `.enqueue()` the tool runs inline and the response is the usual
[`ResultDto`](/transports/http/) with the execution's status. With it, the delivery is
accepted at once with `202` and `{ "job_id", "request_id" }`, and a worker runs the tool
as a [job](/guides/jobs/). **Enqueue is the recommended mode:** providers time out in
seconds and retry, and a job survives a restart and retries on its own.

## The tool

The verified payload is the tool's input. Declare only what you read:

```rust
#[derive(Deserialize, JsonSchema)]
struct StripeEvent {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    data: serde_json::Value,
}

#[carmy::tool(description = "Apply a Stripe subscription event", effect = "write")]
async fn membership_event(State(store): State<Arc<Store>>, input: StripeEvent) -> AgentResult<String> { .. }
```

The tool is an ordinary tool: agents can call it, the console can call it, and its
effect, schema and audit record are the same. A misconfigured webhook (no tool, an
unknown tool, `.enqueue()` without a queue) fails when the app builds, not on the first
delivery.

## Own routes

`Webhook::verify(&headers, &body)` is public, for hosts that mount webhooks on their own
routes and want the same verification.
