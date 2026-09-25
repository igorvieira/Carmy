---
title: Cancelamento e timeouts
description: "Prazos e cancelamento que chegam até o código da sua tool."
sidebar:
  order: 7
---

Agentes abandonam execuções. O Carmy trata isso como um evento de primeira classe, não
como um acidente.

## Prazos

Toda execução tem um prazo: 30 segundos por padrão.

```toml
# carmy.toml
timeout_secs = 10    # ou CARMY_TIMEOUT_SECS=10
```

```rust
carmy::app().timeout(std::time::Duration::from_secs(10)).run().await
```

Uma execução que estoura o prazo retorna `TIMEOUT` (status `timed_out`, HTTP 504).

## O token de cancelamento

Toda execução tem um `CancellationToken` em `ctx.cancellation`. O runtime o cancela e para
de fazer poll da tool quando qualquer uma destas situações acontece:

- o prazo acaba
- o cliente HTTP desconecta, numa requisição JSON ou SSE
- um cliente MCP envia `notifications/cancelled`
- quem chamou descarta a future ou o stream da execução

O trabalho que uma tool dispara em segundo plano deve observar o token:

```rust
#[carmy::tool(effect = "external_write")]
async fn export_report(ctx: AgentContext, input: ExportInput) -> AgentResult<Export> {
    let token = ctx.cancellation.clone();
    let upload = tokio::spawn(async move {
        tokio::select! {
            _ = token.cancelled() => Err("cancelled"),
            result = upload_to_storage(input) => result,
        }
    });
    // …
}
```

## Semântica

Cancelar não desfaz o envio de um e-mail. Por isso o Carmy registra uma execução
interrompida como **incerta**: a reserva de idempotência é mantida, e um retry responde
`EXECUTION_UNCERTAIN` em vez de rodar o efeito colateral duas vezes. Veja
[Idempotência](/pt-br/guides/idempotency/).
