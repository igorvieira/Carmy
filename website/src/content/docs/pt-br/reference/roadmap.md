---
title: Roadmap
description: "O que vem a seguir no Carmy, e o que está fora do escopo."
sidebar:
  order: 6
---

O Carmy está na versão `0.3.0`, e a API é instável durante a série 0.x.

## Próximos passos

- **Planos de execução:** DAGs de chamadas de tools com referências `$step.field`, sobre o
  runtime atual.
- **Eventos de progresso das tools:** progresso e resultados parciais emitidos pelas tools
  no stream de eventos.
- **Transportes:** MCP Streamable HTTP e aprovações via elicitation do MCP.
- **Stores de idempotência:** adaptadores duráveis (Postgres, Redis) em crates separados.
- **Fontes de tools:** adaptadores que transformam uma API GraphQL ou OpenAPI existente
  em tools do Carmy, para que as operações dela ganhem efeitos, confirmação e retries
  seguros antes de um agente encostar nelas. GraphQL não é um transporte: agentes chamam
  tools por MCP e HTTP.

## Fora do escopo

- um runtime assíncrono, parser HTTP ou stack TLS próprios
- um ORM, um scheduler distribuído ou uma engine de workflow completa
- memória de agente, banco vetorial, abstração de LLM, framework de prompts ou roteador de modelos

## Contribuindo

Veja o [CONTRIBUTING.md](https://github.com/igorvieira/Carmy/blob/main/CONTRIBUTING.md). Use
conventional commits e acompanhe mudanças de comportamento com testes de contrato.
