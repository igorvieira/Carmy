---
title: Configuração
description: "carmy.toml, variáveis de ambiente, comandos e features."
sidebar:
  order: 9
---

O `carmy::app()` lê o `carmy.toml` do diretório de trabalho e depois aplica as variáveis de
ambiente. A precedência é: ambiente, depois o arquivo, depois os padrões.

| chave | variável de ambiente | padrão |
|-------|----------------------|--------|
| `name` | `CARMY_NAME` | `carmy` |
| `address` | `CARMY_ADDR` | `127.0.0.1:3000` |
| `timeout_secs` | `CARMY_TIMEOUT_SECS` | `30` |

`CARMY_CONFIG=caminho/para/arquivo.toml` lê outro arquivo. Chaves desconhecidas são
erros, então erros de digitação não passam despercebidos:

```text
invalid configuration: carmy.toml: unknown field `adress`, expected one of `name`, `address`, `timeout_secs`
```

## Comandos

O `run()` escolhe o que fazer pelo primeiro argumento da linha de comando:

| comando | efeito |
|---------|--------|
| *(nenhum)* ou `server` | serve HTTP em `address` |
| `mcp` | serve MCP via stdin/stdout |
| `tools` | imprime o catálogo de tools em JSON e sai |
| `console` | serve `carmy-console/1` pelo stdio (veja [Console](/pt-br/guides/console/)) |

## Builder

Tudo o que está no arquivo também pode ser definido em código, e o código prevalece:

```rust
carmy::app()
    .name("shop")
    .address("0.0.0.0:8080")
    .timeout(std::time::Duration::from_secs(10))
    .state(db)
    .policy(RequireToolPermission)
    .run()
    .await
```

O `Carmy::new()` ignora por completo o arquivo, o ambiente e o registro automático.

## Features

| feature | padrão | oferece |
|---------|--------|---------|
| `http` | sim | `carmy::http`, `.router()`, `.listen()` |
| `mcp` | sim | `carmy::mcp`, `.serve_mcp_stdio()` |
| `observability` | sim | o subscriber de tracing instalado pelo `run()` |

```toml
carmy = { version = "0.2", default-features = false, features = ["http"] }
```
