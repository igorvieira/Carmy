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
| `tool.progress` | a tool reportou progresso ou um resultado parcial; quantas vezes reportar |
| `tool.completed` | a tool terminou, com `duration_ms` e `ok` |
| `execution.completed` | sempre por último, com o resultado completo e `replayed` |

Resultados reenviados e rejeições antecipadas não geram os eventos `tool.*`.

## Progresso

As tools reportam por `ctx.progress`. Sem ninguém ouvindo, a chamada não faz nada, então
reporte à vontade:

```rust
#[carmy::tool(description = "Importa um catálogo", effect = "write")]
async fn import_catalog(ctx: AgentContext, input: Import) -> AgentResult<Imported> {
    let pages = fetch_index(&input.url).await?;
    for (i, page) in pages.iter().enumerate() {
        let items = import_page(page).await?;
        ctx.progress.report((i + 1) as f64, Some(pages.len() as f64), format!("página {}", i + 1));
        ctx.progress.partial(&items);   // um pedaço do resultado que já é útil
    }
    Ok(Imported { pages: pages.len() })
}
```

| onde | o que chega |
|------|-------------|
| SSE | eventos `tool.progress` com `progress`, `total`, `message`, `partial` |
| console | linhas `{"id":…,"event":"progress",…}` antes da resposta; a interface de terminal as mostra na linha de status |
| MCP | `notifications/progress` para chamadas com `progressToken` (sem resultados parciais), e a mensagem de status de uma [task](/pt-br/transports/mcp/#tasks) |
| `execute` | nada: a chamada simples só espera o resultado |

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
