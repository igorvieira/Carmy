---
title: Webhooks
description: "Receba entregas do Stripe, do Telegram ou de qualquer provedor assinado como execuções idempotentes."
sidebar:
  order: 14
---

Um webhook é uma tool com um provedor do outro lado. O Carmy verifica a entrega no corpo
bruto, usa o id do evento do provedor como `request_id` e roda a tool, então uma
reentrega **faz replay** em vez de rodar duas vezes.

```rust
use carmy::http::Webhook;

carmy::app()
    .webhook("/webhooks/stripe", Webhook::stripe(secret).tool("membership_event").enqueue())
    .webhook("/webhooks/telegram", Webhook::telegram(token).tool("telegram_update"))
    .webhook("/webhooks/github", Webhook::hmac_sha256(secret, "X-Hub-Signature-256")
        .tool("github_event")
        .event_id(|body| body["delivery"].as_str().map(str::to_owned)))
```

| construtor | verifica | `request_id` |
|------------|----------|--------------|
| `Webhook::stripe(secret)` | `Stripe-Signature` (`t=`, `v1=`) sobre `"{t}.{body}"`, dentro de 5 minutos (`.tolerance(..)`) | o `id` do evento |
| `Webhook::telegram(token)` | `X-Telegram-Bot-Api-Secret-Token` | `update_id` |
| `Webhook::hmac_sha256(secret, header)` | HMAC-SHA256 do corpo em hex no `header`, com ou sem `sha256=` | nenhum até `.event_id(..)` |

As assinaturas são comparadas em tempo constante. Sem uma válida a tool nunca vê o
corpo: a resposta é `401` com `WEBHOOK_UNAUTHORIZED`. Um corpo que não é JSON responde
`400` com `INVALID_REQUEST`.

## Inline ou enfileirado

Sem `.enqueue()` a tool roda inline e a resposta é o
[`ResultDto`](/pt-br/transports/http/) de sempre, com o status da execução. Com ele, a
entrega é aceita na hora com `202` e `{ "job_id", "request_id" }`, e um worker roda a tool
como um [job](/pt-br/guides/jobs/). **Enfileirar é o modo recomendado:** os provedores
esgotam o tempo em segundos e reenviam, e um job sobrevive a um restart e tenta de novo
sozinho.

## A tool

O payload verificado é a entrada da tool. Declare só o que você lê:

```rust
#[derive(Deserialize, JsonSchema)]
struct StripeEvent {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    data: serde_json::Value,
}

#[carmy::tool(description = "Aplica um evento de assinatura do Stripe", effect = "write")]
async fn membership_event(State(store): State<Arc<Store>>, input: StripeEvent) -> AgentResult<String> { .. }
```

A tool é uma tool comum: agentes podem chamá-la, o console pode chamá-la, e o efeito, o
schema e o registro de auditoria são os mesmos. Um webhook mal configurado (sem tool,
tool desconhecida, `.enqueue()` sem fila) falha quando o app é construído, não na
primeira entrega.

## Rotas próprias

`Webhook::verify(&headers, &body)` é público, para hosts que montam webhooks nas próprias
rotas e querem a mesma verificação.
