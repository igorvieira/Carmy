---
title: Tools
description: "Declare tools tipadas com #[carmy::tool], ou implemente o trait Tool você mesmo."
sidebar:
  order: 1
---

Uma tool é uma `async fn` com o atributo `#[carmy::tool]`:

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

Os tipos de entrada e saída derivam `JsonSchema`, e os doc comments viram descrições no
schema. É assim que os agentes entendem uma tool sem ler a sua documentação.

<small>As aplicações dependem diretamente de `serde` e `schemars`, porque as macros de
derive exigem isso. O `carmy new` já adiciona os dois.</small>

## Assinatura

Uma tool retorna `AgentResult<Output>` e recebe qualquer um dos itens abaixo, em qualquer
ordem:

| parâmetro | significado |
|-----------|-------------|
| `ctx: AgentContext` | opcional; o [contexto do framework](#o-contexto) |
| `State(x): State<T>` | zero ou mais; [dependências da aplicação](/pt-br/guides/state/) |
| `input: Input` | no máximo um; os argumentos. Sem ele, a tool aceita `{}`. |

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

Assinaturas inválidas falham em tempo de compilação, com uma explicação. Por exemplo, dois
inputs geram:

```text
error: tool takes at most one input; combine the fields into one `#[derive(Deserialize, JsonSchema)]` struct
```

## Atributos

| atributo        | valores                                                  | padrão      |
|-----------------|----------------------------------------------------------|-------------|
| `description`   | string                                                   | `""`        |
| `effect`        | `none`, `read`, `write`, `external_write`, `destructive` | obrigatório |
| `idempotent`    | bool                                                     | `false`     |
| `parallel_safe` | bool; `false` serializa as chamadas desta tool           | `false`     |
| `confirmation`  | `none`, `required`                                       | `none`      |
| `register`      | bool; registro automático em `carmy::app()`              | `true`      |

Veja [Efeitos](/pt-br/guides/effects/) para escolher um efeito.

## Registro

Com `carmy::app()`, todo `#[carmy::tool]` do binário se registra sozinho (é coletado em
tempo de link). Para tirar uma tool do registro automático, use `register = false` e
adicione-a explicitamente:

```rust
#[carmy::tool(effect = "read", register = false)]
async fn internal_stats() -> AgentResult<u64> { Ok(0) }

carmy::app().tool(internal_stats).run().await
```

`Carmy::new()` é o builder totalmente explícito: sem arquivo de configuração e sem
registro automático.

```rust
Carmy::new().tool(search).tool(create_order).listen("127.0.0.1:3000").await
```

O registro falha na inicialização se um nome for inválido (1 a 128 letras ASCII, dígitos,
`_`, `-` ou `.`), estiver duplicado ou tiver um schema inválido.

## O contexto

O `AgentContext` guarda apenas dados do framework:

| campo | significado |
|-------|-------------|
| `execution_id` | o ID desta execução |
| `request_id` | a identidade de idempotência, quando o chamador envia uma |
| `session`, `principal` | definidos pelo host depois da autenticação |
| `permissions` | um conjunto de strings concedidas pelo host, como `confirm:cancel_order` |
| `metadata` | valores definidos pelo host |
| `cancellation` | um `CancellationToken`; veja [Cancelamento](/pt-br/guides/cancellation/) |

Ele nunca é um service locator. Coloque dependências em [`State<T>`](/pt-br/guides/state/).

## Implementando `Tool` à mão

A macro gera uma implementação deste trait. Implemente você mesmo quando precisar de
controle total:

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

O código das tools usa dispatch estático. O runtime apaga o tipo uma única vez, no
registro.
