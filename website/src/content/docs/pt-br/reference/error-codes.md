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
| `INVALID_SCHEDULE` | uma expressão de `.schedule(..)` não é cron de sete campos |
| `INVALID_WEBHOOK` | um `.webhook(..)` sem tool, com tool desconhecida, ou `.enqueue()` sem fila |

## Jobs, webhooks e readiness

| código | categoria | quando |
|--------|-----------|--------|
| `JOBS_UNBOUND` | `internal` | o `Jobs` foi usado antes de o app ligá-lo a um runtime |
| `JOBS_CAPACITY` | `capacity` | o store de jobs em memória está cheio |
| `EXECUTION_UNCERTAIN` | `conflict` | a última tentativa de um job estourou o prazo, foi cancelada ou entrou em pânico numa tool não idempotente; ele vai para a DLQ |
| `STORE_ERROR` | `internal` | um store Postgres não conseguiu ler ou escrever; retryable |
| `DATABASE_UNAVAILABLE` | `capacity` | o `carmy::postgres::ready` não obteve resposta do pool |
| `WEBHOOK_UNAUTHORIZED` | `permission` | a assinatura ou o token da entrega está ausente, expirado ou errado (HTTP 401) |
| `READY_TIMEOUT` | `timeout` | um check de readiness levou mais de 5 segundos |
| `WORKER_DOWN` | `capacity` | o `.require_worker(within)` não viu um worker marcar a fila a tempo |
| `UNAVAILABLE` | `not_found` | o `audit` ou `dead` do console sem trilha ou fila anexadas |
