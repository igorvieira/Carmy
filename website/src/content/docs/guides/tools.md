---
title: Tools
description: "Declare typed tools with #[carmy::tool], or implement the Tool trait yourself."
sidebar:
  order: 1
---

A tool is an `async fn` with the `#[carmy::tool]` attribute:

```rust
use carmy::prelude::*;

#[derive(Deserialize, JsonSchema)]
struct CreateOrder {
    /// Product SKU.
    sku: String,
    quantity: u32,
}

#[derive(Serialize, JsonSchema)]
struct Order {
    order_id: u64,
}

#[carmy::tool(description = "Place an order", effect = "write", parallel_safe = true)]
async fn create_order(input: CreateOrder) -> AgentResult<Order> {
    Ok(Order { order_id: 1 })
}
```

The input and output types derive `JsonSchema`, and their doc comments become schema
descriptions. That's how agents understand a tool without reading your docs.

<small>Applications depend on `serde` and `schemars` directly, because their derive macros
require it. `carmy new` adds both.</small>

## Signature

A tool returns `AgentResult<Output>` and takes any of the following, in any order:

| parameter | meaning |
|-----------|---------|
| `ctx: AgentContext` | optional; the [framework context](#the-context) |
| `State(x): State<T>` | zero or more; [application dependencies](/guides/state/) |
| `input: Input` | at most one; the arguments. Without it, the tool accepts `{}`. |

```rust
#[carmy::tool(effect = "read")]
async fn ping() -> AgentResult<String> {
    Ok("pong".into())
}

#[carmy::tool(effect = "read")]
async fn whoami(ctx: AgentContext) -> AgentResult<Option<String>> {
    Ok(ctx.principal)
}
```

Invalid signatures fail at compile time with an explanation. For example, two inputs give:

```text
error: tool takes at most one input; combine the fields into one `#[derive(Deserialize, JsonSchema)]` struct
```

## Attributes

| attribute       | values                                                   | default  |
|-----------------|----------------------------------------------------------|----------|
| `description`   | string                                                   | `""`     |
| `effect`        | `none`, `read`, `write`, `external_write`, `destructive` | required |
| `idempotent`    | bool                                                     | `false`  |
| `parallel_safe` | bool; `false` serializes calls to this tool              | `false`  |
| `confirmation`  | `none`, `required`                                       | `none`   |
| `register`      | bool; auto-registration in `carmy::app()`                | `true`   |

See [Effects](/guides/effects/) for choosing an effect.

## Registration

With `carmy::app()`, every `#[carmy::tool]` in the binary registers itself (it is
collected at link time). To opt a tool out, use `register = false` and add it explicitly:

```rust
#[carmy::tool(effect = "read", register = false)]
async fn internal_stats() -> AgentResult<u64> { Ok(0) }

carmy::app().tool(internal_stats).run().await
```

`Carmy::new()` is the fully explicit builder: no configuration file and no
auto-registration.

```rust
Carmy::new().tool(search).tool(create_order).listen("127.0.0.1:3000").await
```

Registration fails at startup if a name is invalid (1–128 ASCII letters, digits, `_`, `-`
or `.`), is duplicated, or has an invalid schema.

## The context

`AgentContext` holds framework data only:

| field | meaning |
|-------|---------|
| `execution_id` | the ID of this execution |
| `request_id` | the idempotency identity, when the caller sent one |
| `session`, `principal` | set by the host after authentication |
| `permissions` | a set of strings granted by the host, such as `confirm:cancel_order` |
| `metadata` | host-defined values |
| `cancellation` | a `CancellationToken`; see [Cancellation](/guides/cancellation/) |

It is never a service locator. Put dependencies in [`State<T>`](/guides/state/).

## Implementing `Tool` by hand

The macro generates an implementation of this trait. Implement it yourself when you need
full control:

```rust
pub trait Tool: Send + Sync + 'static {
    type Input: DeserializeOwned + JsonSchema + Send + 'static;
    type Output: Serialize + JsonSchema + Send + 'static;
    fn metadata(&self) -> ToolMetadata;
    fn execute(&self, ctx: AgentContext, input: Self::Input)
        -> impl Future<Output = AgentResult<Self::Output>> + Send;
}
```

```rust
struct CreateOrder(Arc<Orders>);

impl Tool for CreateOrder {
    type Input = CreateOrderInput;
    type Output = Order;
    fn metadata(&self) -> carmy::ToolMetadata {
        carmy::ToolMetadata {
            name: "create_order".into(),
            description: "Place an order".into(),
            input_schema: carmy::schema::<CreateOrderInput>(),
            output_schema: carmy::schema::<Order>(),
            effect: Effect::Write,
            idempotent: false,
            parallel_safe: true,
            confirmation: Confirmation::None,
        }
    }
    async fn execute(&self, _ctx: AgentContext, input: CreateOrderInput) -> AgentResult<Order> {
        self.0.place(input)
    }
}
```

Tool code is statically dispatched. The runtime erases the type once, at registration.
