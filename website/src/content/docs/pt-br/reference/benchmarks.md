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

O log por chamada está desligado em todos. Apple M3 Pro, 26/09/2026. Duas execuções
completas ficaram a menos de 3% uma da outra em latência e throughput; cold starts de
poucos milissegundos variam mais. Método, código e resultados brutos:
[`benches/compare`](https://github.com/igorvieira/Carmy/tree/main/benches/compare).

### MCP sobre stdio

| servidor | cold start | p50 | p95 | throughput | p95 sob carga | pico de memória |
|---|---|---|---|---|---|---|
| **Carmy** | 4 ms | 45 µs | 55 µs | 87.100 req/s | 501 µs | 11 MB |
| **Carmy**, `execution_meta(true)` | 4 ms | 49 µs | 58 µs | 66.200 req/s | 664 µs | 11 MB |
| rmcp, o SDK de Rust, sem o Carmy | 3 ms | 52 µs | 62 µs | 40.800 req/s | 916 µs | 7 MB |
| SDK TypeScript | 135 ms | 52 µs | 61 µs | 72.600 req/s | 632 µs | 219 MB |
| SDK Python (`MCPServer`) | 351 ms | 456 µs | 489 µs | 3.100 req/s | 10.604 µs | 65 MB |

### HTTP

| servidor | cold start | p50 | p95 | throughput | p95 sob carga | pico de memória |
|---|---|---|---|---|---|---|
| **Carmy** | 4 ms | 48 µs | 60 µs | 150.200 req/s | 727 µs | 13 MB |
| **Carmy** (1 thread) | 7 ms | 46 µs | 56 µs | 92.300 req/s | 733 µs | 13 MB |
| Axum, sem garantias | 4 ms | 44 µs | 55 µs | 158.500 req/s | 688 µs | 9 MB |
| Axum, sem garantias (1 thread) | 4 ms | 43 µs | 53 µs | 113.000 req/s | 621 µs | 10 MB |
| Express | 132 ms | 65 µs | 79 µs | 44.900 req/s | 1.589 µs | 115 MB |
| FastAPI (uvicorn) | 205 ms | 313 µs | 353 µs | 6.100 req/s | 13.210 µs | 50 MB |

### O que os números dizem

- **MCP:** o Carmy tem o maior throughput e a menor latência entre esses servidores:
  - 87.100 req/s, contra 72.600 do SDK TypeScript.
  - p50 de 45 µs, contra 52 µs do SDK TypeScript e do `rmcp`.
  - O stdio dele usa pipes orientados a prontidão. O `stdio()` padrão do `rmcp` passa
    cada leitura e escrita pelo pool de threads bloqueantes do tokio, por isso o `rmcp`
    puro para em 40.800 req/s.
- **O custo das garantias:**
  - Contra o mesmo tool num handler Axum simples, o Carmy adiciona cerca de 3 a 4 µs no
    p50.
  - Usando todos os núcleos, custa 5% do throughput.
  - Em uma thread custa 18%, contra 38% na 0.1.0. Quando o tool em si quase não custa
    nada, a validação, o prazo e as políticas de cada chamada aparecem.
- **`execution_meta(true)`** coloca `_meta` (ID e status da execução) em toda resposta
  MCP de sucesso. Custa cerca de 24% do throughput com um cliente que faz parse de cada
  objeto a mais, como o do `rmcp`. Vem desligado por padrão, e os erros sempre trazem o
  `_meta`.
- **Memória e cold start:**
  - O Carmy usa de 4 a 20 vezes menos memória que os servidores Node e Python.
  - Sobe em milissegundos, não em centenas de milissegundos.
  - Usa cerca de 4 MB a mais que o `rmcp` ou o Axum puros.
- **Contra Python,** a latência é de 6 a 10 vezes menor e o throughput de 25 a 28 vezes
  maior.
- **Contra Node,** o throughput é 3,3 vezes maior no HTTP e 1,2 vez maior no MCP, com
  latência menor nos dois.

### O que mudou desde a 0.1.0

Cada mudança veio de profiling sob esta carga, e todas mantiveram todas as garantias:

| mudança | efeito |
|---|---|
| stdio do MCP por pipes orientados a prontidão, em vez do pool de threads bloqueantes | throughput MCP +58% |
| O fingerprint de idempotência só é calculado quando há `request_id` | HTTP (1 thread) +4,5% |
| O ID de execução deixou de fazer uma syscall por requisição | HTTP (1 thread) +6% |
| Os eventos de ciclo de vida só são construídos quando alguém faz streaming deles | HTTP (1 thread) +10% |
| O timer de prazo e o waiter de cancelamento só são registrados se o tool suspender | HTTP (1 thread) +6% |
| O adaptador HTTP lê o `Accept` sem clonar todos os cabeçalhos | HTTP (1 thread) +2,7% |
| A metadata de execução em resultados MCP de sucesso virou opt-in | throughput MCP +32% |

No total, o throughput MCP foi de 38.900 para 87.100 req/s (+124%), e o HTTP em uma
thread de 69.600 para 92.300 req/s (+33%). O `cargo bench -p carmy-benches` inclui um
grupo `guarantee/*` com o custo de cada garantia isolada.

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
  um pipe, com o cliente Rust do `rmcp` fazendo as chamadas.
- **Uma máquina.** Rode `benches/compare/run.sh` na sua antes de confiar nesses números.
