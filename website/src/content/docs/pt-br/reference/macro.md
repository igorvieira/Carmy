---
title: "#[carmy::tool]"
description: "Referência da macro de atributo de tools."
sidebar:
  order: 3
---

```rust
#[carmy::tool(
    description = "…",
    effect = "read",          // obrigatório
    idempotent = true,
    parallel_safe = true,
    confirmation = "none",
    register = true,
)]
async fn name(/* ctx, State<T>…, input */) -> AgentResult<Output> { … }
```

## Atributos

| atributo | tipo | padrão |
|----------|------|--------|
| `description` | string | `""` |
| `effect` | `"none"`, `"read"`, `"write"`, `"external_write"` ou `"destructive"` | obrigatório |
| `idempotent` | bool | `false` |
| `parallel_safe` | bool | `false` |
| `confirmation` | `"none"` ou `"required"` | `"none"` |
| `register` | bool | `true` |

## Assinatura

- A função precisa ser `async`, segura e não genérica.
- Ela retorna `AgentResult<Output>`, com `Output: Serialize + JsonSchema`.
- Ela recebe, em qualquer ordem:
  - um `AgentContext` opcional
  - qualquer quantidade de `State<T>`, com `T: Clone + Send + Sync + 'static`
  - no máximo um input, com `Input: Deserialize + JsonSchema`
- Uma função sem input recebe `carmy::NoInput`, que aceita apenas `{}`.

## O que ela gera

- Uma unit struct com o nome da função, que implementa `Tool` (ou `IntoTool` quando a
  função recebe `State`).
- Um `ToolMetadata` com o nome da função, a descrição e os schemas.
- Uma entrada de registro para o `carmy::app()`, a menos que `register = false`.

## Erros de compilação

| erro | mensagem |
|------|----------|
| falta `async` | `tool must be async; a Carmy tool is async fn name(...) -> AgentResult<Output>` |
| dois inputs | `tool takes at most one input; combine the fields into one #[derive(Deserialize, JsonSchema)] struct` |
| efeito desconhecido | `declare effect = "none", "read", "write", "external_write", or "destructive"` |
| atributo desconhecido | `unknown Carmy tool attribute; expected description, effect, …` |
| input sem `JsonSchema` | `the trait bound Input: JsonSchema is not satisfied` |
