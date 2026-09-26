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

## Comparativos

Como o Carmy se compara aos servidores de tools que as pessoas escrevem hoje? Todos os
concorrentes servem **o mesmo tool** (`search_products` sobre um catálogo em memória)
para **o mesmo cliente**. O tool quase não faz trabalho, então os números mostram o que
cada framework adiciona.

```console
benches/compare/run.sh        # precisa de Rust, Node + pnpm e uv
```

Método: cada concorrente roda uma vez para aquecer, com essa rodada descartada, e depois
3 vezes medidas; as tabelas mostram a mediana. Cada rodada mede:

- cold start
- uma checagem de que a resposta está correta
- 5.000 chamadas sequenciais
- 10 segundos de carga: 32 chamadores concorrentes no MCP, 64 no HTTP
- pico de memória

Apple M3 Pro, 26/09/2026. Duas execuções completas ficaram a menos de 5% uma da outra em
latência, throughput e memória. Cold starts de poucos milissegundos variam mais. Método,
código e resultados brutos:
[`benches/compare`](https://github.com/igorvieira/Carmy/tree/main/benches/compare).

### MCP sobre stdio

| servidor | cold start | p50 | p95 | throughput | p95 sob carga | pico de memória |
|---|---|---|---|---|---|---|
| **Carmy** | 4 ms | 62 µs | 71 µs | 38.900 req/s | 966 µs | 10 MB |
| rmcp, o SDK de Rust, sem o Carmy | 2 ms | 53 µs | 61 µs | 40.900 req/s | 886 µs | 7 MB |
| SDK TypeScript | 139 ms | 52 µs | 62 µs | 73.300 req/s | 610 µs | 221 MB |
| SDK Python (`MCPServer`) | 352 ms | 460 µs | 506 µs | 3.100 req/s | 10.774 µs | 65 MB |

### HTTP

| servidor | cold start | p50 | p95 | throughput | p95 sob carga | pico de memória |
|---|---|---|---|---|---|---|
| **Carmy** | 6 ms | 52 µs | 62 µs | 143.200 req/s | 772 µs | 13 MB |
| **Carmy** (1 thread) | 7 ms | 50 µs | 61 µs | 69.600 req/s | 967 µs | 13 MB |
| Axum, sem garantias | 4 ms | 45 µs | 57 µs | 154.400 req/s | 713 µs | 9 MB |
| Axum, sem garantias (1 thread) | 4 ms | 43 µs | 55 µs | 113.000 req/s | 619 µs | 9 MB |
| Express | 135 ms | 64 µs | 77 µs | 44.300 req/s | 1.629 µs | 103 MB |
| FastAPI (uvicorn) | 205 ms | 310 µs | 352 µs | 6.100 req/s | 14.799 µs | 50 MB |

### O que os números dizem

- **As garantias do Carmy custam pouco por chamada.** Contra o mesmo tool num handler
  Axum simples, o Carmy adiciona cerca de 7 µs no p50 e custa 7% do throughput usando
  todos os núcleos. Em uma thread a diferença sobe para 38%, porque o trabalho feito em
  cada chamada aparece quando o tool em si quase não custa nada.
- **Memória e cold start.** O Carmy usa de 4 a 22 vezes menos memória que os servidores
  Node e Python, e sobe em milissegundos em vez de centenas de milissegundos. Isso pesa
  quando você roda muitos servidores de agentes, um por sessão ou por cliente.
- **Contra Python,** a latência é de 6 a 7 vezes menor e o throughput de 12 a 23 vezes
  maior.
- **Contra Node,** o quadro é misto:
  - HTTP: o Carmy tem p50 menor que o Express e 3 vezes o throughput dele.
  - MCP: o SDK TypeScript tem throughput **maior** sob concorrência, 73 mil contra 39
    mil req/s.
  - O `rmcp` de Rust sem o Carmy chega aos mesmos 41 mil, então o teto do MCP aqui é o
    loop de servidor stdio do SDK de Rust, não o Carmy.

### O que os outros não fazem

O custo por chamada compra comportamentos que os outros servidores não oferecem por
padrão:

| embutido | Carmy | os outros, como escritos aqui |
|---|---|---|
| um `request_id` repetido devolve o resultado em vez de repetir o efeito colateral | sim | não |
| efeitos declarados, com tools destrutivos exigindo confirmação confiável | sim | não |
| erros com `recoverable`, `retryable`, `retry_after` e `suggested_action` | sim | não |
| prazo por execução e um token de cancelamento que chega ao tool | sim | não |
| entrada *e saída* validadas contra os schemas declarados | sim | varia |
| o mesmo tool servido por HTTP e MCP | sim | não |
| um span de tracing por execução, sem payloads | sim | não |

### Limites

- **É um microbenchmark de framework.** Tools reais fazem I/O, que costuma ser muito maior
  que esses microssegundos.
- **Node e Python servem em uma thread,** enquanto os servidores Rust usam todos os
  núcleos por padrão. As linhas `(1 thread)` comparam de igual para igual.
- **stdio não é rede.** Os números de MCP medem o enquadramento JSON-RPC e o despacho por
  um pipe.
- **Uma máquina.** Rode `benches/compare/run.sh` na sua antes de confiar nesses números.
