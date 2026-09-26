---
title: Arquitetura
description: "Os crates, o pipeline de execução e as fronteiras entre eles."
sidebar:
  order: 1
---

```text
                  Agente
                    │
          ┌─────────┴─────────┐
        HTTP                 MCP          transportes (carmy-http, carmy-mcp)
          └─────────┬─────────┘
                    │  ExecutionRequest
               Carmy Runtime              policy · validação · idempotência ·
                    │                     prazo · tracing
          ┌─────────┼─────────┐
        Tool      Tool      Tool          o seu código
          │         │         │
       Serviço   Banco de   API externa
                  dados
```

## Crates

| crate | papel |
|-------|-------|
| `carmy` | a fachada: `carmy::app()`, `State`, configuração, `testing`, `prelude` e transportes ativados por features |
| `carmy-core` | o domínio: `Tool`, `ToolMetadata`, `Effect`, `AgentError`, `AgentContext`, tipos de execução |
| `carmy-schema` | geração de JSON Schema |
| `carmy-macros` | `#[carmy::tool]` |
| `carmy-runtime` | registro, policies, validação, idempotência, cancelamento, stream de eventos, sinks de auditoria |
| `carmy-http` | descoberta, catálogo, execução e SSE sobre o Axum; webhooks, `/health` e `/ready` |
| `carmy-jobs` | tools que rodam depois: fila, retries, dead letters, agendamentos, worker |
| `carmy-postgres` | stores duráveis: jobs (com outbox), idempotência e auditoria |
| `carmy-mcp` | o adaptador de servidor MCP sobre o `rmcp` |
| `carmy-observability` | configuração do subscriber de tracing |
| `carmy-cli` | `carmy new`, `carmy g tool`, `carmy console`, `carmy server` |

O `carmy-core` não depende de HTTP, MCP, bancos de dados nem provedores de LLM.

## O pipeline de execução

`Runtime::execute(ExecutionRequest) -> ExecutionResult`:

1. Resolve a tool pelo nome.
2. Roda as policies: autorização, confirmação, cotas.
3. Valida os IDs e valida os argumentos contra o schema de entrada.
4. Reserva a identidade de idempotência: adquirir, reenviar, conflito ou incerto.
5. Invoca a tool sob um prazo, o token de cancelamento e isolamento de panics.
6. Valida a saída contra o schema de saída.
7. Registra o resultado.
8. Fecha o span de tracing.

O `Runtime::execute_stream` roda o mesmo pipeline e emite eventos.

## Transportes

Um transporte só traduz:

1. o seu protocolo para um `ExecutionRequest`, com o `AgentContext` confiável do host
2. o seu sinal de cancelamento para o token do contexto
3. resultados e eventos para os seus próprios DTOs

Os tipos do runtime nunca vão direto para o transporte. Adicionar um transporte não exige
nenhuma mudança nas tools nem no core.

## As convenções moram na fachada

`State<T>`, o registro automático (coletado em tempo de link com `linkme`), o `carmy.toml`
e o `run()` ficam todos no crate `carmy`. O core e o runtime continuam livres deles.

## Rumo aos planos de execução

Um futuro plano (um DAG de chamadas de tools com referências `$step.field`) não precisa de
conceitos novos:

- Cada passo é um `ExecutionRequest`.
- `parallel_safe` decide quais passos podem rodar em paralelo.
- `effect` e `idempotent` decidem o que pode ser repetido ou guardado em cache.
- `request_id`s por passo (`<plan>/<step>`) tornam seguro retomar um plano que falhou.

## Checklist de revisão

Confira em toda mudança:

- O conceito faz parte do domínio ou de um transporte?
- HTTP ou MCP estão vazando para o runtime ou para o core?
- Estado da aplicação está vazando para o `AgentContext`?
- A nova abstração é necessária agora, ou apenas permitida pela arquitetura?
- Ela impede execution plans ou um novo transporte no futuro?
- Ela torna o caso comum mais difícil?
