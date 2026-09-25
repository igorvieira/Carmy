---
title: Observabilidade
description: "Spans de tracing para cada execução, sem vazar payloads."
sidebar:
  order: 11
---

Toda execução roda dentro de um span de tracing `carmy.execution` com estes campos:

| campo | valor |
|-------|-------|
| `execution_id` | o ID da execução |
| `request_id` | a identidade de idempotência, quando existe |
| `tool` | o nome da tool |
| `effect` | o efeito declarado |
| `status` | `completed`, `failed`, `cancelled` ou `timed_out` |
| `duration_ms` | a duração total |
| `replayed` | se o resultado foi reenviado |
| `error_code` | o código de erro, em caso de falha |

Um evento de conclusão é registrado em `INFO` no sucesso e em `WARN` na falha. **Argumentos
e saídas nunca são registrados**, porque podem conter segredos.

```text
INFO carmy.execution{execution_id=exec_… request_id="abc" tool=create_order effect="write" status="completed" duration_ms=0 replayed=true}: execution finished
```

## Configuração

O `carmy::run()` instala um subscriber que escreve no stderr e é filtrado por `RUST_LOG`
(padrão `info`). O stdout fica livre para o MCP via stdio.

## OpenTelemetry

O Carmy não depende de OpenTelemetry. Componha as camadas você mesmo, e os spans são
exportados com os mesmos campos:

```rust
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

tracing_subscriber::registry()
    .with(EnvFilter::new("info"))
    .with(carmy::observability::fmt_layer())
    .with(tracing_opentelemetry::layer().with_tracer(tracer))
    .init();
```

Instale o seu subscriber antes de chamar `run()`. Se já existir um subscriber global, o
Carmy o mantém.
