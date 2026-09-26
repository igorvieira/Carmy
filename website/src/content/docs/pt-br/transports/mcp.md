---
title: MCP
description: "Sirva as mesmas tools para clientes MCP como o Claude Desktop e o Cursor."
sidebar:
  order: 2
---

O `carmy-mcp` adapta o runtime ao Model Context Protocol usando o SDK oficial em Rust,
o [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk). As suas tools não mudam.

```console
cargo run -- mcp
```

## Conectando um cliente

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

## Suportado na v0.1

| recurso do MCP | suporte |
|----------------|---------|
| `initialize`, `ping` | sim |
| `tools/list` | sim, com o catálogo inteiro em uma página e `ttlMs` de 60000 |
| `tools/call` | sim |
| `notifications/cancelled` | sim; cancela a execução |
| transporte stdio | sim |
| outros transportes do `rmcp` | sim, via `rmcp::ServiceExt::serve` |
| resources, prompts, completion | não |
| sampling, elicitation, roots | não |
| notificações de progresso, tasks | não |

## Mapeamento

| Carmy | MCP |
|-------|-----|
| `effect` `none` ou `read` | `readOnlyHint: true` |
| `effect` `destructive` | `destructiveHint: true` |
| `effect` `external_write` | `openWorldHint: true` |
| `idempotent` | `idempotentHint` |
| efeito, confirmação, segurança para paralelismo | `_meta["carmy/effect"]`, `_meta["carmy/confirmation"]`, `_meta["carmy/parallel_safe"]` |
| resultado de sucesso em objeto | `structuredContent`, mais conteúdo de texto em JSON |
| `AgentError` | `isError: true`, com `{"error": …}` em JSON no conteúdo de texto e em `_meta["carmy/error"]`; nunca em `structuredContent`, que o cliente valida contra o output schema |
| tool desconhecida | JSON-RPC `-32602`, com `data.error` |
| `_meta["carmy/request_id"]` no `tools/call` | o `request_id` de idempotência |
| ID e status da execução | `_meta["carmy/execution_id"]` e `_meta["carmy/status"]` no resultado: sempre nos erros, e nos sucessos com `McpServer::execution_meta(true)` |

## Configuração explícita

```rust
let runtime = Carmy::new().tool(search).build()?;
carmy::mcp::McpServer::new(runtime)
    .context(trusted_context) // ex.: permissões de confirmação decididas pelo host
    .serve_stdio()
    .await?;
```

## Limitações

- O MCP espera schemas de entrada do tipo objeto, então tools cuja entrada não é um objeto
  são listadas como estão.
- Tools que exigem confirmação precisam de um `confirm:<tool>` concedido pelo host no
  contexto da conexão. Ainda não existe um fluxo de aprovação por chamada via elicitation.
