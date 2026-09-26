---
title: Webhooks
description: "Receba entregas assinadas de qualquer remetente como execuções idempotentes de tools."
sidebar:
  order: 14
---

Um webhook é uma tool com um remetente do outro lado. O Carmy verifica a entrega no corpo
bruto, lê a identidade dela no payload como `request_id` e roda a tool, então uma
reentrega **faz replay** em vez de rodar duas vezes.

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

O Carmy não conhece provedor nenhum. Ele oferece três verificações genéricas, e o app diz
o header, o segredo e onde fica a identidade:

| construtor | verifica |
|------------|----------|
| `Webhook::hmac_sha256(secret, header)` | o `header` traz o HMAC-SHA256 do corpo em hex, com ou sem o prefixo `sha256=` |
| `Webhook::shared_secret(header, secret)` | o `header` é igual a um segredo compartilhado com o remetente |
| `Webhook::custom(\|headers, body\| ..)` | qualquer outro esquema: assinaturas com timestamp, outros algoritmos, allow-lists |

Os segredos são comparados em tempo constante. Sem uma entrega válida a tool nunca vê o
corpo: a resposta é `401` com `WEBHOOK_UNAUTHORIZED`. Um corpo que não é JSON responde
`400` com `INVALID_REQUEST`.

## Identidade

`.event_id("/id")` é um [JSON pointer](https://datatracker.ietf.org/doc/html/rfc6901)
dentro do payload: `/id`, `/event/id`, `/update_id`. Strings e números viram o
`request_id`. Sem ele, ou quando o pointer não encontra nada, as entregas não são
deduplicadas e todas rodam.

## Inline ou enfileirado

Sem `.enqueue()` a tool roda inline e a resposta é o
[`ResultDto`](/pt-br/transports/http/) de sempre, com o status da execução. Com ele, a
entrega é aceita na hora com `202` e `{ "job_id", "request_id" }`, e um worker roda a tool
como um [job](/pt-br/guides/jobs/). **Enfileirar é o modo recomendado:** os remetentes
esgotam o tempo em segundos e reenviam, e um job sobrevive a um restart e tenta de novo
sozinho.

## Um esquema próprio

Um remetente que assina `"{timestamp}.{body}"` e rejeita timestamps antigos cabe no
`Webhook::custom`:

```rust
Webhook::custom(move |headers, body| {
    let Some((t, signature)) = parse_signature_header(headers) else { return false };
    fresh(t, Duration::from_secs(300)) && hmac_matches(&secret, &[t.as_bytes(), b".", body], signature)
})
```

## A tool

O payload verificado é a entrada da tool. Declare só o que você lê:

```rust
#[derive(Deserialize, JsonSchema)]
struct BillingEvent {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    data: serde_json::Value,
}

#[carmy::tool(description = "Aplica um evento de assinatura", effect = "write")]
async fn billing_event(State(store): State<Arc<Store>>, input: BillingEvent) -> AgentResult<String> { .. }
```

A tool é uma tool comum: agentes podem chamá-la, o console pode chamá-la, e o efeito, o
schema e o registro de auditoria são os mesmos. Um webhook mal configurado (sem tool,
tool desconhecida, `.enqueue()` sem fila) falha quando o app é construído, não na
primeira entrega.

## Rotas próprias

`Webhook::verify(&headers, &body)` é público, para hosts que montam webhooks nas próprias
rotas e querem a mesma verificação.
