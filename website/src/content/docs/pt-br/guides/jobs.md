---
title: Jobs
description: "Rode tools depois: uma fila com retries, dead letters e agendamentos."
sidebar:
  order: 13
---

Parte do trabalho não deve acontecer dentro de uma requisição: publicar em três canais,
revalidar uma oferta um dia depois, coletar ofertas a cada cinco minutos. O `carmy::jobs`
roda tools depois. Um job é um `ExecutionRequest` com `request_id`, então ele passa pelo
mesmo runtime, com as mesmas policies, validação, idempotência e auditoria de uma
chamada direta.

## Enfileirar

As tools recebem `State<Jobs>`; o app o insere antes de registrá-las.

```rust
use carmy::{jobs::Jobs, prelude::*, runtime::execution_request};

#[carmy::tool(description = "Passa uma oferta pela pipeline", effect = "write")]
async fn process_offer(State(jobs): State<Jobs>, input: OfferInput) -> AgentResult<Processed> {
    let deal = build_deal(input.offer)?;
    jobs.enqueue(
        execution_request("publish_premium", json!({ "deal_id": deal.id }))
            .with_request_id(format!("publish-{}-premium", deal.id)),
    )
    .await?;
    jobs.enqueue_after(
        execution_request("publish_public", json!({ "deal_id": deal.id }))
            .with_request_id(format!("publish-{}-public", deal.id)),
        Duration::from_secs(30 * 60),
    )
    .await?;
    Ok(deal.into())
}
```

| método | efeito |
|--------|--------|
| `enqueue(request)` | roda assim que um worker estiver livre |
| `enqueue_after(request, delay)` / `enqueue_at(request, when)` | roda depois |
| `every(name, cron, make)` | um agendamento; veja abaixo |
| `cancel(id)` | cancela um job que ainda não começou; `true` quando cancelou |
| `get(id)` | o job, com status, tentativas e último erro |
| `dead_letters(limit)` | jobs que desistiram |

**O `request_id` é a identidade do job.** Enfileirar a mesma identidade enquanto um job
com ela está na fila ou rodando devolve o id desse job em vez de duplicar. Uma coleta
que roda duas vezes, ou um webhook entregue duas vezes, nunca processa duas vezes.

## Worker

```rust
carmy::app().jobs(store).run().await   // `cargo run -- worker`
```

O `worker` reivindica os jobs vencidos e os roda, `[jobs].concurrency` por vez (padrão
4). Cada job tem um **lease** que o worker renova enquanto a tool roda; um job cujo lease
expirou (o worker morreu) é reivindicado por outro worker. Em `SIGTERM` ou Ctrl-C o
worker para de reivindicar, termina os jobs em andamento e sai.

`Jobs::run_due(limit)` roda uma rodada na task atual, para testes e para hosts com loop
próprio.

## Retries

O erro decide. Veja [Erros](/pt-br/guides/errors/).

| resultado | o que acontece |
|-----------|----------------|
| completou | `succeeded` |
| erro com `retryable: true` | nova tentativa, após um backoff exponencial que respeita o `retry_after`, até `max_attempts` (padrão 5) |
| erro com `retryable: false` | `failed`; sem novas tentativas |
| `TIMEOUT`, `CANCELLED`, `TOOL_PANIC` | **incerto**: a tool pode ter efetivado. Nova tentativa só se a tool for `idempotent`; senão `dead_lettered` com `EXECUTION_UNCERTAIN` |
| tentativas esgotadas | `dead_lettered` com o último erro |

Cada tentativa roda com `request_id = "{id}#{tentativa}"`, porque o runtime já registrou
a anterior. Um job na DLQ é uma decisão para uma pessoa ou um agente: o `carmy console`
lista esses jobs com `dead`.

```rust
carmy::app().retry(RetryPolicy { max_attempts: 3, base: Duration::from_secs(2), cap: Duration::from_secs(300) })
```

## Agendamentos

```rust
carmy::app()
    .schedule("collect", "0 */5 * * * * *", || execution_request("collect_offers", json!({})))
```

As expressões cron têm sete campos, segundos primeiro. Cada ocorrência vira um job com
`request_id = "collect@{timestamp}"`, então vários workers marcando o mesmo agendamento o
enfileiram uma vez só. Um worker que ficou fora enfileira as ocorrências perdidas no
último minuto, e nada além disso.

## Stores

O `InMemoryJobStore` é o padrão: local ao processo, limitado, para desenvolvimento e
testes. Em produção use o `carmy::postgres::PostgresJobStore` (feature `postgres`):

```rust
let pool = carmy::postgres::connect(&url).await?;
carmy::postgres::migrate(&pool).await?;
carmy::app().jobs(Arc::new(PostgresJobStore::new(pool.clone())))
```

Os workers reivindicam com `FOR UPDATE SKIP LOCKED`, então qualquer número deles
compartilha a fila. O store também oferece um **outbox transacional**:
`store.enqueue_in(&mut tx, request, run_at, max_attempts)` insere o job dentro da sua
transação, para que ele exista exatamente quando a escrita que ele acompanha for
efetivada.

Qualquer implementação de `JobStore` serve; a trait tem sete métodos.
