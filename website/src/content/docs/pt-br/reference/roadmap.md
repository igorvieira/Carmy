---
title: Roadmap
description: "O que vem a seguir no Carmy, e o que está fora do escopo."
sidebar:
  order: 6
---

O Carmy está na versão `0.6.1`, e a API é instável durante a série 0.x.

## Próximos passos

- **GraphQL como fonte de tools:** operações de um schema GraphQL e de um arquivo de
  operações viram tools, como as operações [OpenAPI](/pt-br/guides/openapi/). GraphQL não é
  um transporte: agentes chamam tools por MCP e HTTP.
- **OpenAPI em YAML**, ao lado do JSON.
- **Próximas ações vindas das tools:** tools sugerindo os próprios próximos passos, ao lado
  dos que o Carmy já conhece.
- **Tasks do MCP duráveis:** tasks do MCP apoiadas em [jobs](/pt-br/guides/jobs/), para
  sobreviverem a um restart.
- **Auditoria no Redis**, ao lado do Postgres.

Planos de execução (DAGs de chamadas de tools) não estão mais previstos: a composição
fica com o agente, ou com tools que enfileiram outras tools.

## Fora do escopo

- um runtime assíncrono, parser HTTP ou stack TLS próprios
- um ORM
- uma engine de workflow: sem DAGs de passos, sagas ou estado guardado entre passos. Os
  [jobs](/pt-br/guides/jobs/) do Carmy rodam **uma chamada de tool depois**, com retries e
  agendamentos, e param aí
- um scheduler distribuído além disso: sem eleição de líder nem membership de cluster; os
  locks de linha do banco são a única coordenação
- memória de agente, banco vetorial, abstração de LLM, framework de prompts ou roteador de modelos

## Contribuindo

Veja o [CONTRIBUTING.md](https://github.com/igorvieira/Carmy/blob/main/CONTRIBUTING.md). Use
conventional commits e acompanhe mudanças de comportamento com testes de contrato.
