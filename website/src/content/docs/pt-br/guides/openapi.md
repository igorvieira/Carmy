---
title: OpenAPI
description: "Transforme uma API existente em tools com efeitos, confirmação e retries seguros."
sidebar:
  order: 19
---

Uma API que você já tem vira tools que um agente usa com segurança. O `carmy::openapi`
(feature `openapi`) lê uma descrição OpenAPI 3.0 ou 3.1 e cria uma tool por operação,
cada uma com efeito, regra de confirmação, schema de entrada validado e retries
seguros, antes de qualquer agente encostar na API.

```toml
carmy = { version = "0.6", features = ["openapi"] }
```

```rust
let github = carmy::openapi::OpenApi::from_file("github.json")?
    .base_url("https://api.github.com")
    .header("Authorization", format!("Bearer {}", std::env::var("GITHUB_TOKEN")?))
    .only(["issues/list-for-repo", "issues/create"])
    .prefix("github_")
    .tools()?;

carmy::app().tools(github).run().await
```

## O que cada operação vira

| na descrição | na tool |
|--------------|---------|
| `operationId` | o nome (com `.prefix(..)`; caracteres que o Carmy não aceita viram `_`) |
| `summary`, `description` | a descrição que os agentes leem |
| parâmetros de path e query | propriedades da entrada; os de path são obrigatórios |
| o corpo JSON da requisição | a propriedade `body` da entrada |
| schemas de componentes referenciados | os `$defs` da própria tool, só o que ela usa |
| `nullable: true` do OpenAPI 3.0 | um tipo que admite `null` |

Parâmetros de header e cookie ficam de fora: carregam credenciais e encanamento, que o
host define com `.header(..)`, nunca o agente.

O método define o efeito:

| método | efeito | idempotente | confirmação |
|--------|--------|-------------|-------------|
| `GET`, `HEAD` | `read` | sim | não |
| `PUT` | `external_write` | sim | não |
| `POST`, `PATCH` | `external_write` | não | não |
| `DELETE` | `destructive` | sim | exigida |

Uma operação que lê com `POST` (uma busca, por exemplo) diz isso com
`x-carmy-effect: read` na descrição, ou `.effect("searchPets", Effect::Read)` no código.

## Chamadas

- A entrada é validada contra o schema antes de qualquer coisa sair do processo.
- Valores de path são codificados, `/` incluída, então nunca saem do seu segmento.
- O `request_id` da chamada vai no header `Idempotency-Key` nas escritas, então uma API
  que o respeita também deduplica os retries.
- Os headers do host vão em toda chamada; os agentes nunca os veem.

As respostas da API viram [erros](/pt-br/guides/errors/) sobre os quais um agente
consegue agir:

| resposta da API | código | retry |
|-----------------|--------|-------|
| 429 | `UPSTREAM_RATE_LIMITED` | sim, depois do `Retry-After` |
| 503, outros 5xx | `UPSTREAM_UNAVAILABLE` | sim |
| 401, 403 | `UPSTREAM_UNAUTHORIZED` | não |
| 404 | `UPSTREAM_NOT_FOUND` | não |
| 409 | `UPSTREAM_CONFLICT` | não |
| outros 4xx | `UPSTREAM_REJECTED` | não |
| sem conexão | `UPSTREAM_UNREACHABLE` | sim: a requisição nunca saiu |
| sem resposta | `UPSTREAM_FAILED` | só se a tool for idempotente |

O `details` traz o status e o corpo (os primeiros 2 KiB de um corpo em texto).

## Respostas

As respostas não são validadas contra a descrição por padrão: APIs se afastam dos seus
documentos, e um campo novo não deveria fazer falhar uma chamada que deu certo. O
`.strict_output(true)` as valida contra os schemas 2xx, e mostra esses schemas aos
agentes.

## Opções

| método | faz |
|--------|-----|
| `.base_url(url)` | onde a API está; padrão: a primeira URL de `servers` |
| `.header(nome, valor)` | um header em toda chamada |
| `.only([..])` | só esses `operationId`s |
| `.prefix(p)` | prefixo de todos os nomes de tool |
| `.effect(id, efeito)` | sobrescreve o efeito de uma operação |
| `.strict_output(true)` | valida as respostas |
| `.client(reqwest::Client)` | proxies, timeouts, configurações de TLS |

Por enquanto só descrições em JSON; converta YAML com qualquer ferramenta antes.
