---
title: Redis
description: "Jobs e idempotência duráveis no Redis, compartilhados por todas as instâncias de um serviço."
sidebar:
  order: 18
---

O `carmy::redis` (feature `redis`) guarda a fila de jobs e os registros de idempotência
no Redis. Escolha-o quando o Redis é o que você já roda; escolha o
[Postgres](/pt-br/guides/postgres/) quando quiser a trilha de auditoria durável também,
ou o outbox transacional.

```toml
carmy = { version = "0.6", features = ["redis"] }
```

```toml
# carmy.toml
[database]
url = "redis://localhost:6379"   # rediss:// para TLS; CARMY_DATABASE_URL ou DATABASE_URL
idempotency_retention_days = 7
```

É só isso. O `carmy::app()` então:

- guarda jobs e registros de idempotência no Redis;
- adiciona o check `database` ao `/ready` (um `PING`);
- adiciona os comandos `migrate` (nada a fazer) e `cleanup`;
- injeta a conexão nas tools que recebem `State<carmy::redis::Redis>`.

Com Redis, a trilha de auditoria fica em memória; ela continua alimentando o `carmy
console` e a `carmy_audit`.

## Como funciona

| chave | guarda |
|-------|--------|
| `{carmy}:job:<id>` | um hash: o job, o status, as tentativas |
| `{carmy}:queue` | jobs na fila, por `run_at` |
| `{carmy}:running` | jobs rodando, pelo fim do lease |
| `{carmy}:dead`, `{carmy}:finished` | dead letters e jobs terminados, por horário |
| `{carmy}:active:<request_id>` | o job ativo com essa identidade, para enfileirar duas vezes devolver o mesmo |
| `{carmy}:idem:<identidade>` | um registro de idempotência: fingerprint, resultado |

- **Atômico.** Toda mudança de vários passos (enfileirar, reivindicar, terminar, cancelar,
  reservar, completar) é um script Lua. Os workers nunca dividem um job.
- **O relógio do Redis manda nos leases.** Um claim ou um heartbeat define o lease pelo
  `TIME` do Redis, então workers com relógios dessincronizados concordam sobre quando um
  dono sumiu. Os horários de execução seguem o relógio do app.
- **Um slot de hash.** Toda chave carrega o hash tag `{carmy}`, então um Redis Cluster as
  mantém juntas, como os scripts precisam.
- **Falha rápido.** A conexão abre no primeiro uso, reconecta sozinha e desiste de uma
  chamada em segundos quando o Redis está fora: o `/ready` responde, e a chamada falha
  com um `STORE_ERROR` retryable.

## Retenção

Registros de idempotência completos **expiram** depois de `idempotency_retention_days`
(sete por padrão); os em andamento nunca expiram, porque marcam execuções que podem ter
efetivado. Um `request_id` repetido depois que o registro expirou roda de novo, então
mantenha a retenção maior que qualquer retry dos clientes.

Jobs terminados ficam até o `cleanup` remover os mais antigos que `jobs_retention_days`
(trinta por padrão). Dead letters sempre ficam.

```console
$ cargo run -- cleanup
{"jobs":5310}
```

## Peça por peça

```rust
use carmy::redis::{Redis, RedisIdempotencyStore, RedisJobStore};

let redis = Redis::open("redis://localhost")?;   // ainda não conecta
carmy::app()
    .jobs(Arc::new(RedisJobStore::new(redis.clone())))
    .idempotency_store(Arc::new(
        RedisIdempotencyStore::new(redis).with_retention(Duration::from_secs(3 * 24 * 3600)),
    ))
```
