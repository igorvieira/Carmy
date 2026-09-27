---
title: Postgres
description: "Jobs, idempotência e auditoria duráveis, compartilhados por todas as instâncias de um serviço."
sidebar:
  order: 17
---

Os stores padrão ficam em memória: um restart os perde, e duas instâncias de um serviço
não enxergam o estado uma da outra. O `carmy::postgres` (feature `postgres`) guarda a
fila de jobs, os registros de idempotência e a trilha de auditoria no Postgres. Cada
peça é opcional.

```toml
carmy = { version = "0.5", features = ["postgres"] }
```

Depois, dê a ele uma URL, como qualquer outra configuração:

```toml
# carmy.toml
[database]
url = "postgres://localhost/shop"   # CARMY_DATABASE_URL, depois DATABASE_URL, têm prioridade
```

É só isso. O `carmy::app()` então:

- guarda jobs, registros de idempotência e a trilha de auditoria no Postgres;
- migra as tabelas do Carmy antes de qualquer comando, exceto `tools`;
- adiciona o check `database` ao `/ready`;
- adiciona os comandos `migrate` e `cleanup`;
- injeta o pool nas tools que recebem `State<PgPool>`, e o store de jobs nas que recebem
  `State<PostgresJobStore>` (para o outbox).

```rust
use carmy::postgres::sqlx::PgPool;

#[carmy::tool(description = "Cria um pedido", effect = "write")]
async fn create_order(State(pool): State<PgPool>, input: NewOrder) -> AgentResult<Order> {
    sqlx::query_as("INSERT INTO orders (sku) VALUES ($1) RETURNING id, sku")
        .bind(input.sku)
        .fetch_one(&pool)
        .await
        .map_err(|e| AgentError::new("DB", e.to_string(), ErrorCategory::Internal).retryable(Some(1)))
}
```

O pool conecta no primeiro uso, então um banco fora do ar no start faz o `/ready` falhar,
não o processo. Em código, `Carmy::database(url)` faz o mesmo que a configuração.

### Peça por peça

Cada store também funciona sozinho, sobre qualquer `sqlx::PgPool`:

```rust
use carmy::postgres::{PostgresAudit, PostgresIdempotencyStore, PostgresJobStore, connect, migrate};

let pool = connect(&url).await?;   // 8 conexões, 5 s para obter uma
migrate(&pool).await?;
carmy::app()
    .jobs(Arc::new(PostgresJobStore::new(pool.clone())))
    .idempotency_store(Arc::new(PostgresIdempotencyStore::new(pool.clone())))
    .sink(Arc::new(PostgresAudit::new(pool.clone())))
```

## Migrations

O `migrate` cria e atualiza as tabelas `carmy_*`. É seguro rodá-lo em todo start e em
várias instâncias ao mesmo tempo: elas se revezam num advisory lock. O Carmy registra as
versões dele na própria tabela `carmy_schema_version` e nunca toca na
`_sqlx_migrations`, então convive com as migrations do seu app sem conflito.

| tabela | guarda |
|--------|--------|
| `carmy_jobs` | a fila de jobs |
| `carmy_idempotency` | uma linha por identidade de requisição, com fingerprint e resultado |
| `carmy_audit` | uma linha por execução, nunca argumentos nem saídas |
| `carmy_schema_version` | quais migrations do Carmy rodaram |

## Jobs

- **Enfileirar nunca duplica.** Um índice único parcial em `request_id` cobre os jobs na
  fila e rodando; enfileirar uma identidade já ativa devolve esse job.
- **Os workers dividem uma fila.** O claim usa `SELECT … FOR UPDATE SKIP LOCKED`: cada
  worker pula as linhas que outro segura, então nenhum job roda duas vezes e nenhum
  worker espera.
- **Os leases usam o relógio do banco.** Um job reivindicado pertence ao worker até
  `now() + lease` no banco, renovado enquanto a tool roda. Workers em máquinas com
  relógios dessincronizados concordam sobre quando um dono sumiu. Os horários de
  execução (`run_at`) seguem o relógio do app.
- **O outbox transacional** enfileira dentro da sua transação, então o job existe
  exatamente quando a escrita que ele acompanha é efetivada:

```rust
let mut tx = pool.begin().await?;
sqlx::query("INSERT INTO orders …").execute(&mut *tx).await?;
store.enqueue_in(&mut tx, request, chrono::Utc::now(), 5).await?;
tx.commit().await?;   // o pedido e o job existem juntos, ou nenhum dos dois
```

O outbox recebe uma transação do `sqlx`. Um app com outra biblioteca de banco ainda usa
todos os stores, mas não o outbox.

## Idempotência

As reservas são um `INSERT … ON CONFLICT DO NOTHING`, então duas instâncias recebendo o
mesmo `request_id` ao mesmo tempo concordam sobre um único dono. A semântica é a do
store em memória: replay, `IDEMPOTENCY_CONFLICT`, `EXECUTION_UNCERTAIN`. Veja
[Idempotência](/pt-br/guides/idempotency/).

## Auditoria

As execuções nunca esperam o banco: os registros passam por um canal limitado até uma
task que escreve. Quando o canal enche, um registro é descartado e um aviso vai para o
log. Veja [Auditoria](/pt-br/guides/audit/).

## Retenção

Nada é apagado sozinho. O `cleanup` remove o que um `Retention` não guarda mais, em
lotes de 5000 linhas, com idades pelo relógio do banco:

| tipo | padrão | sempre mantidos |
|------|--------|-----------------|
| jobs terminados (succeeded, failed, cancelled) | 30 dias | jobs na fila, rodando e na DLQ |
| registros de idempotência completos | 7 dias | registros em andamento, que marcam execuções incertas |
| registros de auditoria | 90 dias | |

Um `request_id` repetido depois que o registro de idempotência dele sumiu **roda de
novo**, então mantenha essa retenção maior que qualquer retry dos clientes.

Com um banco configurado, o comando `cleanup` executa a limpeza. Agende o comando com
cron, um CronJob do Kubernetes ou qualquer agendador:

```console
$ cargo run -- cleanup
{"audit":1204,"idempotency":88,"jobs":5310}
```

```toml
[database]
jobs_retention_days = 30
idempotency_retention_days = 7
audit_retention_days = 90
```

Em código: `.retention(Retention { .. })`, ou `carmy::postgres::cleanup(&pool, retention)`
em qualquer lugar.

O `Jobs::purge(older_than)` faz a parte dos jobs por qualquer `JobStore`, inclusive o em
memória.

## Readiness

O `carmy::postgres::ready(pool)` roda `SELECT 1`. Atrás do `/ready`, ele tira uma
instância do load balancer quando o banco dela está inacessível. Veja
[Readiness e rotas](/pt-br/guides/readiness/).
