---
title: MCP
description: "Sirva as mesmas tools para clientes MCP como o Claude Desktop e o Cursor, por stdio ou HTTP."
sidebar:
  order: 2
---

O `carmy-mcp` adapta o runtime ao Model Context Protocol usando o SDK oficial em Rust,
o [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk). As suas tools não mudam.

## Por stdio

```console
cargo run -- mcp
```

Gere um binário de release e aponte o seu cliente MCP para ele. Por exemplo, na
configuração do Claude Desktop:

```json
{
  "mcpServers": {
    "shop": { "command": "/path/to/shop/target/release/shop", "args": ["mcp"] }
  }
}
```

Os logs vão para o stderr, e o stdout fica reservado para o protocolo.

## Por HTTP

Com as features `http` e `mcp` (ambas ligadas por padrão), o app serve MCP por
Streamable HTTP em `/mcp`, ao lado das rotas de agente e atrás dos mesmos limites de
conexão e timeouts. A descoberta anuncia o endpoint como `mcp_url`.

```toml
[mcp]
http = true                          # false: só stdio
path = "/mcp"
allowed_hosts = ["api.example.com"]  # CARMY_MCP_ALLOWED_HOSTS=a,b
```

O `allowed_hosts` protege contra DNS rebinding conferindo o header `Host`. Sem ele, só
localhost é aceito, então **liste seus hosts públicos em produção**.

As chamadas usam o `AgentContext` que o seu middleware de autenticação colocou na
requisição, o mesmo das chamadas [HTTP](/pt-br/transports/http/#embutindo-o-router):

```rust
let router = carmy::app().router()?.layer(from_fn(authenticate)); // insere Extension<AgentContext>
```

## Progresso

Quando um `tools/call` traz um `progressToken`, o que a tool reporta por
[`ctx.progress`](/pt-br/guides/streaming/#progresso) chega como `notifications/progress`,
com o progresso, o total e a mensagem. Resultados parciais não fazem parte do progresso
do MCP; eles ficam nos streams do próprio Carmy (SSE, console).

## Tasks

Com clientes que suportam [tasks](https://modelcontextprotocol.io) (a extensão
`io.modelcontextprotocol/tasks`), uma chamada que ainda roda depois de `promote_after`
vira uma task: o cliente recebe um id de task na hora, acompanha com `tasks/get` e pode
cancelar com `tasks/cancel`. As mensagens de progresso da tool viram a mensagem de
status da task. Chamadas rápidas, e clientes sem tasks, respondem inline como sempre.

```toml
[mcp]
promote_after_ms = 2000   # padrão; 0 nunca promove
```

Essas tasks vivem no processo por cinco minutos. Para trabalho que precisa sobreviver a
um restart, enfileire um [job](/pt-br/guides/jobs/) e acompanhe com a `carmy_job`.

## Confirmação

Uma tool que exige confirmação (destrutiva, ou com `confirmation = "required"`) pergunta
à pessoa por trás do cliente, por um formulário de elicitation: "`delete_customer`
precisa da sua confirmação antes de rodar com {…}. Permitir?". Com um sim, a chamada roda
com `confirm:<tool>` só para ela. Clientes que não fazem elicitation recebem o erro
`CONFIRMATION_REQUIRED` de sempre. Desligue a pergunta quando a resposta do cliente não
puder valer:

```toml
[mcp]
confirm_by_elicitation = false
```

## Suportado

| recurso do MCP | suporte |
|----------------|---------|
| `initialize`, `ping` | sim |
| `tools/list` | sim, o catálogo inteiro numa página, `ttlMs` de 60000 |
| `tools/call` | sim |
| `notifications/cancelled` | sim; cancela a execução |
| `notifications/progress` | sim, para chamadas com `progressToken` |
| tasks (`tasks/get`, `tasks/cancel`) | sim, para chamadas que passam de `promote_after` |
| elicitation | sim, para confirmar tools que exigem |
| stdio, Streamable HTTP | sim |
| resources, prompts, completion, sampling | não |

## Mapeamento

| Carmy | MCP |
|-------|-----|
| `effect` `none` ou `read` | `readOnlyHint: true` |
| `effect` `destructive` | `destructiveHint: true` |
| `effect` `external_write` | `openWorldHint: true` |
| `idempotent` | `idempotentHint` |
| efeito, confirmação, segurança em paralelo | `_meta["carmy/effect"]`, `_meta["carmy/confirmation"]`, `_meta["carmy/parallel_safe"]` |
| resultado de sucesso que é objeto | `structuredContent`, mais conteúdo de texto em JSON |
| `AgentError` | `isError: true`, com `{"error": …}` como texto JSON e em `_meta["carmy/error"]`; nunca em `structuredContent`, que os clientes validam contra o schema de saída |
| tool desconhecida | JSON-RPC `-32602`, com `data.error` |
| `_meta["carmy/request_id"]` no `tools/call` | o `request_id` de idempotência |
| ID e status da execução | `_meta["carmy/execution_id"]` e `_meta["carmy/status"]`: sempre nos erros, nos sucessos com `McpServer::execution_meta(true)` |
| [próximas ações](/pt-br/transports/http/#próximas-ações) | `_meta["carmy/next_actions"]`, sempre que houver |

## Configuração explícita

```rust
let runtime = Carmy::new().tool(search).build()?;
let server = carmy::mcp::McpServer::new(runtime)
    .context(trusted_context)          // ex.: concessões decididas pelo host
    .promote_after(Some(Duration::from_secs(5)))
    .confirm_by_elicitation(true);
server.clone().serve_stdio().await?;                    // stdio
let service = server.http_service(Some(vec!["api.example.com".into()]));
let router = axum::Router::new().nest_service("/mcp", service);  // HTTP, feature `http`
```

## Limitações

- O MCP espera schemas de entrada que sejam objetos, então tools cuja entrada não é um
  objeto aparecem como estão.
- As tasks ficam no processo; um restart as perde.
- As sessões MCP por HTTP ficam em memória: atrás de um load balancer, mantenha cada
  cliente numa instância (sticky sessions), ou use MCP por stdio.
