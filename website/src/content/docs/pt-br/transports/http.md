---
title: HTTP
description: "O protocolo HTTP carmy/1: descoberta, catálogo, execução e SSE."
sidebar:
  order: 1
---

O adaptador HTTP serve três endpoints sobre o Axum. Todos os corpos são JSON, e o corpo
das requisições é limitado a 1 MiB.

## `GET /.well-known/agent`

```json
{
  "protocol": "carmy/1",
  "server": "shop",
  "capabilities": ["tools", "streaming", "idempotency"],
  "tools_url": "/agent/tools",
  "tools_version": "3762507c…",
  "execute_url": "/agent/execute"
}
```

O `tools_version` só muda quando o catálogo muda, então os agentes podem guardar o
catálogo em cache em vez de baixá-lo de novo. Um app com [webhooks](/pt-br/guides/webhooks/)
acrescenta `"webhooks"` a `capabilities` e lista cada um (caminho, tool, modo, pointer de
identidade), nunca com o segredo. Com [MCP por HTTP](/pt-br/transports/mcp/#por-http), o
`mcp_url` aponta para ele e `capabilities` inclui `"mcp"`.

## `GET /agent/tools`

```json
{
  "tools": [
    {
      "name": "create_order",
      "description": "Place an order",
      "input_schema": { "type": "object", "properties": { "sku": { "type": "string" } }, "required": ["sku"] },
      "output_schema": { "type": "object", "properties": { "order_id": { "type": "integer" } } },
      "effect": "write",
      "idempotent": false,
      "parallel_safe": true,
      "confirmation": "none"
    }
  ]
}
```

Os dois documentos de descoberta são serializados uma única vez, na inicialização, e
servidos com `ETag` e `Cache-Control: private, max-age=60`. Uma requisição com
`If-None-Match` recebe `304 Not Modified` quando o documento não mudou.

## `POST /agent/execute`

```json
{ "tool": "create_order", "arguments": { "sku": "KB-01" }, "request_id": "order-7f3" }
```

- `arguments` tem `{}` como padrão.
- `request_id` é opcional. Envie em tudo o que você puder [repetir](/pt-br/guides/idempotency/).
- Campos desconhecidos são rejeitados. Uma requisição nunca pode carregar contexto nem
  permissões.

```json
{
  "execution_id": "exec_…",
  "status": "completed",
  "data": { "order_id": 1 },
  "_agent": { "cacheable": false, "next_actions": [] }
}
```

- `status` é um de `completed`, `failed`, `cancelled` ou `timed_out`.
- As falhas trazem [`error`](/pt-br/guides/errors/) no lugar de `data`.
- `_agent.cacheable` é `true` quando um resultado pode ser reaproveitado: uma chamada bem
  sucedida a uma tool idempotente com efeito `read` ou `none`.
- `_agent.next_actions` lista o que fazer em seguida, quando o Carmy sabe; veja abaixo.
- As respostas são enviadas com `Cache-Control: no-store`.

### Próximas ações

```json
"_agent": { "cacheable": false, "next_actions": [
  { "tool": "publish", "reason": "Retry the same call with the same request_id", "after_ms": 5000 }
] }
```

| depois de | o Carmy sugere |
|-----------|----------------|
| um erro retryable | a mesma chamada, mesmo `request_id`, depois do `retry_after` |
| `CONFIRMATION_REQUIRED` | a mesma chamada, depois que uma pessoa confirmar |
| `carmy_job` de um job ainda na fila ou rodando | `carmy_job` de novo, em um segundo |
| uma entrega de webhook enfileirada (`202`) | `carmy_job` com o novo `job_id` |

Sem `arguments` quer dizer os mesmos argumentos. Uma lista vazia quer dizer que não há o
que sugerir. A mesma lista vai nas respostas do console e no
`_meta["carmy/next_actions"]` dos resultados do MCP.

### Status codes

| categoria | status | | categoria | status |
|-----------|--------|-|-----------|--------|
| `validation` | 400 | | `conflict` | 409 |
| `permission` | 403 | | `internal` | 500 |
| `not_found` | 404 | | `capacity` | 503 |
| `cancelled` | 408 | | `timeout` | 504 |

O corpo é o que vale; o status code é uma cortesia para ferramentas HTTP genéricas. Corpos
malformados retornam `400 INVALID_REQUEST` com `details.reason`, e corpos grandes demais
retornam `413 PAYLOAD_TOO_LARGE`.

## Streaming

Adicione `Accept: text/event-stream` para receber [Server-Sent Events](/pt-br/guides/streaming/),
inclusive o `tool.progress` da tool. Se o cliente desconectar, a execução é cancelada.

## MCP

Com a feature `mcp`, o mesmo router serve [MCP por HTTP](/pt-br/transports/mcp/#por-http)
em `/mcp`; a descoberta o lista como `mcp_url`.

## Embutindo o router

```rust
let router: axum::Router = carmy::app().router()?;
// adicione suas rotas e middlewares Tower, e sirva com axum::serve
```
