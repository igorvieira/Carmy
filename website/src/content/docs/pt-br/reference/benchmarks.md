---
title: Benchmarks
description: "Como o overhead do Carmy é medido."
sidebar:
  order: 5
---

```console
cargo bench -p carmy-benches
```

A tool usada nos benchmarks não faz trabalho nenhum, então cada número é o overhead do
próprio Carmy por cima do seu código.

| benchmark | mede |
|-----------|------|
| `dispatch/direct_tool_call` | referência: chamar a tool tipada diretamente |
| `dispatch/runtime_execute` | o pipeline completo do runtime |
| `json/result_dto` | serializar um resultado HTTP |
| `discovery/schema_generation` | gerar metadados e schemas |
| `discovery/tool_catalog_json` | serializar o catálogo |
| `http/execute_roundtrip` | uma requisição Axum no próprio processo, a chamada ao runtime e a resposta |
| `concurrent/64_read_only_executions` | 64 execuções concorrentes |

Uma execução local (Apple M3 Pro, rustc 1.93.0):

| benchmark | mediana |
|-----------|---------|
| `dispatch/direct_tool_call` | 90 ns |
| `dispatch/runtime_execute` | 2,5 µs |
| `json/result_dto` | 0,31 µs |
| `discovery/schema_generation` | 2,1 µs |
| `discovery/tool_catalog_json` | 2,1 µs |
| `http/execute_roundtrip` | 4,7 µs |
| `concurrent/64_read_only_executions` | 98 µs |

Esses números descrevem uma máquina; rode os benchmarks de novo antes de confiar neles. O
Carmy não faz nenhuma afirmação de desempenho que esses benchmarks não consigam reproduzir.
