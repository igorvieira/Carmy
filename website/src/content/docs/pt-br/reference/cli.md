---
title: CLI
description: "O comando carmy e os comandos da aplicação."
sidebar:
  order: 2
---

## `carmy`

```console
cargo install carmy-cli
```

O Carmy exige Rust **1.88** ou mais recente (`rustup update stable`).

| comando | efeito |
|---------|--------|
| `carmy new <name>` | cria uma aplicação com a [estrutura convencional](/pt-br/getting-started/project-structure/) |
| `carmy new <name> --git` | depende da branch `main` do repositório Git em vez do crates.io |
| `carmy new <name> --path <dir>` | depende de um checkout local do Carmy em vez do crates.io |
| `carmy generate tool <name> --effect <efeito>` | adiciona uma tool à aplicação atual |
| `carmy g tool …` | atalho para `carmy generate tool` |
| `carmy --version` | imprime a versão |
| `carmy --help` | imprime o uso |

O `carmy new`:

- valida o nome: letras ASCII minúsculas, dígitos, `_` e `-`, começando por uma letra;
  nomes reservados são recusados
- nunca sobrescreve um diretório existente
- roda `git init` quando o Git está disponível

## `carmy generate tool`

```console
carmy g tool create_order --effect write --description "Place an order"
```

Ele cria `src/tools/create_order.rs`, com as structs tipadas de entrada e saída, a
função com `#[carmy::tool]` e um teste, e declara `mod create_order;` em
`src/tools/mod.rs`, o que registra a tool. Funciona de qualquer diretório dentro da
aplicação.

| opção | significado |
|-------|-------------|
| `--effect` | **obrigatória**: `none`, `read`, `write`, `external_write` ou `destructive` |
| `--description` | o que a tool faz; os agentes leem isso |

Os atributos seguem o efeito:

| efeito | atributos |
|--------|-----------|
| `none`, `read` | `idempotent` e `parallel_safe` |
| `destructive` | `confirmation = "required"`, e o teste gerado confere que a confirmação é exigida |

O `carmy g tool` nunca sobrescreve um arquivo, recusa nomes que não sejam snake_case (e
palavras reservadas do Rust) e formata o código com o `rustfmt` quando ele está
instalado.

## Comandos da aplicação

Uma aplicação Carmy construída com `carmy::run()` aceita:

| comando | efeito |
|---------|--------|
| `cargo run` ou `cargo run -- server` | serve HTTP |
| `cargo run -- mcp` | serve MCP via stdio |
| `cargo run -- tools` | imprime o catálogo de tools em JSON |

Os mesmos comandos funcionam no binário compilado: `./target/release/shop mcp`.
