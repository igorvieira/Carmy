---
title: Estrutura do projeto
description: "A estrutura convencional de uma aplicação Carmy."
sidebar:
  order: 3
---

O `carmy new` cria esta estrutura:

```text
shop/
├── Cargo.toml
├── carmy.toml          # nome, endereço, timeout; variáveis CARMY_* sobrescrevem
├── README.md
├── .gitignore
└── src/
    ├── main.rs         # o ponto de entrada
    └── tools/
        ├── mod.rs      # uma linha `mod` por tool
        └── hello.rs    # uma tool e seus testes
```

## `src/main.rs`

```rust
mod tools;

#[tokio::main]
async fn main() -> carmy::Result {
    carmy::run().await
}
```

`carmy::run()` é um atalho para `carmy::app().run().await`. O `carmy::app()` faz três
coisas:

- lê a [configuração](/pt-br/guides/configuration/) do `carmy.toml` e do ambiente
- registra toda tool declarada com `#[carmy::tool]`
- escolhe o transporte pelo primeiro argumento da linha de comando

Para registrar dependências compartilhadas, use o builder:

```rust
#[tokio::main]
async fn main() -> carmy::Result {
    let db = Db::connect("postgres://localhost/shop").await.expect("database");
    carmy::app().state(db).run().await
}
```

## `src/tools/`

A convenção é um arquivo por tool, declarado em `src/tools/mod.rs`. O gerador faz as
duas coisas por você:

```console
carmy g tool search --effect read --description "Search the catalog"
```

O resultado fica assim:

```rust
//! Um arquivo por tool. Declare cada arquivo aqui; `#[carmy::tool]` o registra.
mod hello;
mod search;
```

Mantenha os testes de uma tool no mesmo arquivo (veja [Testes](/pt-br/guides/testing/)).

## `carmy.toml`

```toml
name = "shop"                # CARMY_NAME
address = "127.0.0.1:3000"   # CARMY_ADDR
timeout_secs = 30            # CARMY_TIMEOUT_SECS
```

## Comandos

| comando | efeito |
|---------|--------|
| `cargo run` ou `cargo run -- server` | serve HTTP no endereço configurado |
| `cargo run -- mcp` | serve MCP via stdin/stdout |
| `cargo run -- tools` | imprime o catálogo de tools em JSON |
| `cargo run -- console` ou `carmy console` | explora e chama as tools (veja [Console](/pt-br/guides/console/)) |
| `cargo test` | roda os testes |
