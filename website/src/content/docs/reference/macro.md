---
title: "#[carmy::tool]"
description: "Reference for the tool attribute macro."
sidebar:
  order: 3
---

```rust
#[carmy::tool(
    description = "…",
    effect = "read",          // required
    idempotent = true,
    parallel_safe = true,
    confirmation = "none",
    register = true,
)]
async fn name(/* ctx, State<T>…, input */) -> AgentResult<Output> { … }
```

## Attributes

| attribute | type | default |
|-----------|------|---------|
| `description` | string | `""` |
| `effect` | `"none"`, `"read"`, `"write"`, `"external_write"` or `"destructive"` | required |
| `idempotent` | bool | `false` |
| `parallel_safe` | bool | `false` |
| `confirmation` | `"none"` or `"required"` | `"none"` |
| `register` | bool | `true` |

## Signature

- The function must be `async`, safe and non-generic.
- It returns `AgentResult<Output>`, where `Output: Serialize + JsonSchema`.
- It takes, in any order:
  - an optional `AgentContext`
  - any number of `State<T>`, where `T: Clone + Send + Sync + 'static`
  - at most one input, where `Input: Deserialize + JsonSchema`
- A function without an input gets `carmy::NoInput`, which accepts only `{}`.

## What it generates

- A unit struct named after the function, which implements `Tool` (or `IntoTool` when
  the function takes `State`).
- `ToolMetadata` with the function name, the description and the schemas.
- A registration entry for `carmy::app()`, unless `register = false`.

## Compile errors

| mistake | message |
|---------|---------|
| missing `async` | `tool must be async; a Carmy tool is async fn name(...) -> AgentResult<Output>` |
| two inputs | `tool takes at most one input; combine the fields into one #[derive(Deserialize, JsonSchema)] struct` |
| unknown effect | `declare effect = "none", "read", "write", "external_write", or "destructive"` |
| unknown attribute | `unknown Carmy tool attribute; expected description, effect, …` |
| an input without `JsonSchema` | `the trait bound Input: JsonSchema is not satisfied` |
