---
title: Introdução
description: "O que é o Carmy e por que agentes precisam de uma camada de execução determinística."
sidebar:
  order: 1
---

O Carmy é uma **infraestrutura de execução nativa para agentes, em Rust**. Ele não é mais
um framework HTTP de uso geral: não roteia páginas web e não compete com Axum, Actix ou
Hyper. Ele é construído sobre eles.

> O raciocínio de um LLM pode ser probabilístico. Os efeitos colaterais, não.

## Por que agentes precisam disso

Agentes chamam tools de um jeito que clientes web comuns não chamam:

- **Eles repetem às cegas.** Depois de um timeout, de uma falha de rede ou de um retry do
  modelo, o agente chama de novo, muitas vezes sem saber se a primeira tentativa foi
  efetivada.
- **Eles leem máquinas, não documentação.** Agentes descobrem tools por schemas e
  metadados, e reagem a códigos de erro, não a texto.
- **Eles desistem no meio.** Execuções são abandonadas, e o trabalho em segundo plano não
  pode continuar rodando com semântica indefinida.
- **Eles precisam de proteções.** Uma tool que apaga dados precisa ser identificável, e
  bloqueada, *antes* de rodar.

O Carmy torna cada um desses pontos explícito no runtime, para que toda tool tenha as
mesmas garantias.

## Conceitos principais

| conceito | o que é |
|----------|---------|
| **Tool** | Uma função Rust tipada com metadados: nome, descrição, schemas, efeito, idempotência, segurança para paralelismo e confirmação. |
| **Execução** | Uma execução de uma tool. Tem um `execution_id`, um `request_id` opcional para retries, um status e um resultado. |
| **Efeito** | O efeito colateral declarado de uma tool: `none`, `read`, `write`, `external_write` ou `destructive`. |
| **Contexto** | `AgentContext`: dados confiáveis do framework (principal, sessão, permissões, cancelamento). Não guarda estado da aplicação. |
| **Policy** | Um hook que roda antes de toda execução: autorização, confirmação, cotas. |
| **Transporte** | Um adaptador que transforma um protocolo (HTTP, MCP) em execuções do runtime. |

## As invariantes

- O core não conhece transportes, e as tools não conhecem HTTP.
- O runtime não conhece provedores de LLM.
- Os efeitos são explícitos, e os erros são legíveis por máquina.
- Os retries são seguros, e a execução é observável.
- Os transportes são substituíveis.

## O que o Carmy não é

O Carmy não tem runtime assíncrono próprio, parser HTTP, TLS, ORM, engine de workflow,
memória de agente, banco vetorial, abstração de LLM nem framework de prompts. Esses são
outros problemas.

Próximo: [Início rápido](/pt-br/getting-started/quick-start/).
