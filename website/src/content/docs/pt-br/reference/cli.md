---
title: CLI
description: "O comando carmy e os comandos da aplicação."
sidebar:
  order: 2
---

## `carmy`

```console
cargo install --git https://github.com/igorvieira/Carmy carmy-cli
```

| comando | efeito |
|---------|--------|
| `carmy new <name>` | cria uma aplicação com a [estrutura convencional](/pt-br/getting-started/project-structure/) |
| `carmy new <name> --path <dir>` | depende de um checkout local do Carmy em vez do repositório Git |
| `carmy --version` | imprime a versão |
| `carmy --help` | imprime o uso |

O `carmy new`:

- valida o nome: letras ASCII minúsculas, dígitos, `_` e `-`, começando por uma letra;
  nomes reservados são recusados
- nunca sobrescreve um diretório existente
- roda `git init` quando o Git está disponível

## Comandos da aplicação

Uma aplicação Carmy construída com `carmy::run()` aceita:

| comando | efeito |
|---------|--------|
| `cargo run` ou `cargo run -- server` | serve HTTP |
| `cargo run -- mcp` | serve MCP via stdio |
| `cargo run -- tools` | imprime o catálogo de tools em JSON |

Os mesmos comandos funcionam no binário compilado: `./target/release/shop mcp`.
