---
title: Efeitos e confirmação
description: "Declare efeitos colaterais explicitamente e proteja tools destrutivas com confirmação confiável."
sidebar:
  order: 3
---

Toda tool declara o seu efeito. O Carmy nunca o infere a partir de nomes ou métodos HTTP.

| efeito | significado | exemplo |
|--------|-------------|---------|
| `none` | computação pura | formatação, cálculos |
| `read` | lê estado, não altera nada | busca, consulta |
| `write` | altera o seu sistema | criar um pedido |
| `external_write` | altera um sistema que você não controla | enviar um e-mail, cobrar um cartão |
| `destructive` | irreversível ou destrói dados | apagar um cliente |

Os efeitos são publicados na descoberta (`GET /agent/tools`) e nas annotations de tools do
MCP, para que clientes e hosts possam aplicar políticas de aprovação *antes* de chamar uma
tool.

## Confirmação

```rust
#[carmy::tool(
    description = "Delete a customer permanently",
    effect = "destructive",
    confirmation = "required"
)]
async fn delete_customer(input: CustomerId) -> AgentResult<()> { /* … */ Ok(()) }
```

A policy padrão, `SafePolicy`, rejeita tools destrutivas e tools que declaram
`confirmation = "required"`, a menos que o contexto confiável conceda `confirm:<tool>`:

```json
{ "error": { "code": "CONFIRMATION_REQUIRED", "category": "permission", "recoverable": false, "retryable": false, "message": "Trusted confirmation is required" } }
```

A permissão vem do **host**, nunca dos argumentos da tool nem do corpo da requisição. Em
geral, a sua interface pergunta a um humano, e o seu middleware de autenticação insere a
permissão:

```rust
ctx.permissions.insert("confirm:delete_customer".into());
```

Veja [Segurança](/pt-br/guides/security/) para saber onde o host define o contexto.

## Segurança para paralelismo e idempotência

- `parallel_safe = false` (o padrão) serializa as execuções daquela tool. Use `true`
  quando chamadas concorrentes forem seguras.
- `idempotent = true` declara que repetir a chamada é inofensivo. Junto com um efeito
  `read` ou `none`, isso permite ao Carmy marcar os resultados como `cacheable` para os
  clientes.
