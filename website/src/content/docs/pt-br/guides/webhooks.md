---
title: Webhooks
description: "Uma porta para uma tool: entregas verificadas de qualquer remetente, rodadas como execuções idempotentes."
sidebar:
  order: 14
---

Um webhook é uma porta para uma tool. O Carmy verifica a entrega, lê a identidade dela no
payload como `request_id` e roda a tool, então uma reentrega **faz replay** em vez de
rodar duas vezes. O Carmy não conhece remetente nenhum: você diz como verificar uma
entrega.

```rust
use carmy::http::{Webhook, verify};

carmy::app()
    .webhook("/webhooks/billing", Webhook::to("billing_event")
        .verify(verify::hmac_sha256(secret, "X-Signature"))
        .event_id("/id")
        .enqueue())
```

| método | faz |
|--------|-----|
| `Webhook::to(tool)` | a tool que recebe o payload verificado como entrada |
| `.verify(check)` | como reconhecer uma entrega autêntica; obrigatório |
| `.event_id("/id")` | um [JSON pointer](https://datatracker.ietf.org/doc/html/rfc6901) para a identidade da entrega; strings e números viram o `request_id` |
| `.enqueue()` | responde `202` na hora e roda a tool como um [job](/pt-br/guides/jobs/) |
| `.unverified()` | aceita toda entrega, para remetentes autenticados antes (um gateway, uma rede privada) |

## Verificação

O `.verify` recebe qualquer função de um `Delivery` (os headers e o corpo bruto) para
`bool`. Duas verificações comuns já vêm prontas:

| verificação | aceita quando |
|-------------|---------------|
| `verify::hmac_sha256(secret, header)` | o `header` traz o HMAC-SHA256 do corpo em hex, com ou sem `sha256=` |
| `verify::shared_secret(header, secret)` | o `header` é igual a um segredo compartilhado com o remetente |

Qualquer outra coisa é uma closure. Compare segredos em tempo constante:

```rust
Webhook::to("partner_event").verify(move |delivery| {
    let Some(signature) = delivery.header("x-partner-signature") else { return false };
    my_scheme::is_valid(&secret, signature, delivery.body)
})
```

Uma entrega que falha na verificação nunca chega à tool: `401` com
`WEBHOOK_UNAUTHORIZED`. Um corpo que não é JSON responde `400` com `INVALID_REQUEST`.

## Seguro por padrão

O app se recusa a subir, com `INVALID_WEBHOOK` e o motivo, quando um webhook:

- não tem `.verify(..)` nem `.unverified()`;
- aponta para uma tool que não existe;
- usa `.enqueue()` sem fila de jobs;
- usa um caminho já ocupado, ou que não começa com `/`.

Sem `.event_id(..)`, ou quando o pointer não encontra nada, as entregas não são
deduplicadas e todas rodam.

## Inline ou enfileirado

Sem `.enqueue()` a tool roda inline e a resposta é o
[`ResultDto`](/pt-br/transports/http/) de sempre. Com ele, a resposta é `202` com
`{ "job_id", "request_id" }`, e um worker roda a tool. **Enfileirar é o modo
recomendado:** os remetentes esgotam o tempo em segundos e reenviam, e um job sobrevive a
um restart e tenta de novo sozinho.

## Para agentes

Os webhooks aparecem na descoberta e no console (`webhooks`), nunca com os segredos:

```json
"webhooks": [
  { "path": "/webhooks/billing", "tool": "billing_event", "mode": "enqueue", "event_id": "/id", "verified": true }
]
```

Um agente que precisa reprocessar ou simular uma entrega chama a tool direto com o
payload e o mesmo `request_id`. É o mesmo caminho, com o mesmo efeito, replay e
auditoria, e sem assinatura para forjar.

## Rotas próprias

`webhook.check(&headers, &body)` é público, para hosts que montam webhooks nas próprias
rotas e querem a mesma verificação.
