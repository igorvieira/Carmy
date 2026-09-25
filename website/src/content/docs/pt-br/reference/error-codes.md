---
title: Códigos de erro
description: "Códigos de erro produzidos pelo próprio Carmy."
sidebar:
  order: 4
---

As suas tools definem os próprios códigos. Estes são os códigos que o Carmy produz:

## Execução

| código | categoria | quando |
|--------|-----------|--------|
| `TOOL_NOT_FOUND` | `not_found` | nenhuma tool tem esse nome |
| `INVALID_ARGUMENTS` | `validation` | os argumentos não batem com o schema de entrada |
| `INVALID_ID` | `validation` | um execution ID vazio, ou um `request_id` com mais de 256 bytes |
| `CONFIRMATION_REQUIRED` | `permission` | uma tool destrutiva ou que exige confirmação, sem `confirm:<tool>` |
| `FORBIDDEN` | `permission` | `RequireToolPermission` está ativa e falta `tool:<name>` |
| `IDEMPOTENCY_CONFLICT` | `conflict` | um `request_id` reutilizado com outra tool ou outros argumentos |
| `EXECUTION_UNCERTAIN` | `conflict` | um `request_id` cuja execução está rodando ou foi interrompida |
| `IDEMPOTENCY_CAPACITY` | `capacity` | o store de idempotência em memória está cheio |
| `TIMEOUT` | `timeout` | o prazo acabou; efeitos externos podem ter sido efetivados |
| `CANCELLED` | `cancelled` | a execução foi cancelada; efeitos externos podem ter sido efetivados |
| `TOOL_PANIC` | `internal` | a tool entrou em panic; efeitos externos podem ter sido efetivados |
| `INVALID_OUTPUT` | `internal` | a saída não bate com o schema de saída, ou não pode ser serializada |

## HTTP

| código | status | quando |
|--------|--------|--------|
| `INVALID_REQUEST` | 400 | o corpo não é uma requisição de execução válida; veja `details.reason` |
| `PAYLOAD_TOO_LARGE` | 413 | o corpo passa de 1 MiB |

## Inicialização

Estes são retornados por `build()`, `run()` e `listen()` como `carmy::Error::Registration`:

| código | quando |
|--------|--------|
| `INVALID_TOOL_NAME` | um nome não tem de 1 a 128 letras ASCII, dígitos, `_`, `-` ou `.` |
| `DUPLICATE_TOOL` | duas tools têm o mesmo nome |
| `INVALID_SCHEMA` | um schema de entrada ou de saída não é um JSON Schema válido |
| `MISSING_STATE` | uma tool precisa de um `State<T>` que nunca foi registrado |
