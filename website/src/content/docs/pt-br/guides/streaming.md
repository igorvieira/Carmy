---
title: Streaming
description: "Acompanhe execuções como um stream de eventos de ciclo de vida, no próprio processo ou via SSE."
sidebar:
  order: 6
---

O runtime expõe toda execução como um stream de eventos, independente de formato de
transporte:

| evento | quando |
|--------|--------|
| `execution.started` | sempre primeiro |
| `tool.started` | a tool vai rodar (depois das policies, da validação e da idempotência) |
| `tool.completed` | a tool terminou, com `duration_ms` e `ok` |
| `execution.completed` | sempre por último, com o resultado completo e `replayed` |

Resultados reenviados e rejeições antecipadas não geram os eventos `tool.*`.

## Via HTTP (SSE)

Envie a requisição normal de execução com `Accept: text/event-stream`:

```console
$ curl -N localhost:3000/agent/execute -H 'content-type: application/json' \
    -H 'accept: text/event-stream' -d '{"tool":"search","arguments":{"query":"mouse"}}'
event: execution.started
data: {"execution_id":"exec_…","tool":"search"}

event: tool.started
data: {"execution_id":"exec_…","tool":"search"}

event: tool.completed
data: {"duration_ms":0,"execution_id":"exec_…","ok":true,"tool":"search"}

event: execution.completed
data: {"_agent":{"cacheable":true,"next_actions":[]},"data":{…},"execution_id":"exec_…","replayed":false,"status":"completed"}
```

O payload de `execution.completed` é o mesmo corpo de uma resposta JSON, mais `replayed`.

## No próprio processo

```rust
use futures_util::StreamExt;

let runtime = carmy::app().build()?;
let request = carmy::runtime::execution_request("search", serde_json::json!({ "query": "x" }));
let mut events = runtime.execute_stream(request);
while let Some(event) = events.next().await {
    println!("{}", event.name());
}
```

O próprio stream conduz a execução; nada é disparado em segundo plano. Descartar o stream
(por exemplo, quando um cliente SSE desconecta) cancela a execução, exatamente como
descartar uma requisição comum.
