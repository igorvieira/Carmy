---
title: Auditoria
description: "Quem rodou o quê, quando, e como terminou, sem argumentos nem saídas."
sidebar:
  order: 15
---

Toda execução deixa um `ExecutionRecord`: replays, rejeições de policy e falhas de
validação incluídas. Os registros nunca carregam argumentos nem saídas, que podem conter
segredos, então a trilha é segura de guardar e de mostrar.

```json
{
  "execution_id": "exec_d8df…",
  "request_id": "publish-kb-01-premium",
  "principal": "worker",
  "session": null,
  "tool": "publish_premium",
  "effect": "external_write",
  "status": "completed",
  "error_code": null,
  "duration_ms": 12,
  "started_at": "2026-09-26T14:03:11.204Z",
  "replayed": false
}
```

## Sinks

Um `ExecutionSink` recebe cada registro na task da execução, então precisa retornar na
hora. O runtime aceita quantos forem:

```rust
carmy::app()
    .sink(Arc::new(PostgresAudit::new(pool)))   // durável; feature `postgres`
```

| sink | guarda |
|------|--------|
| `InMemoryAudit` | os últimos 1000 registros; todo app tem um, e o `carmy console` o lê |
| `carmy::postgres::PostgresAudit` | a tabela `carmy_audit`, escrita por uma task atrás de um canal limitado; sob pressão ele descarta um registro e loga um aviso em vez de atrasar execuções |

Um sink próprio é uma trait com um método: `fn record(&self, record: ExecutionRecord)`.

## Lendo a trilha

No `carmy console`, ou pelo `carmy-console/1`:

```text
audit 20            -> os últimos registros, mais novos primeiro
dead                -> os jobs na fila de dead letters
```

`PostgresAudit::recent(limit)` e `InMemoryAudit::recent(limit)` devolvem os mesmos
registros em código. Uma execução com replay tem `replayed: true` e o `execution_id`
**original**, então um webhook reentregue aparece como dois registros apontando para uma
execução.
