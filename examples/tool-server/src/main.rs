//! One set of tools, two transports.
//!
//! ```text
//! cargo run -p tool-server            # HTTP on 127.0.0.1:3000 (CARMY_ADDR to change)
//! cargo run -p tool-server -- --mcp   # MCP over stdio
//! ```
use carmy::prelude::*;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Deserialize, JsonSchema)]
struct SearchInput {
    /// Search expression, matched case-insensitively against product names.
    query: String,
}
#[derive(Serialize, JsonSchema)]
struct SearchOutput {
    products: Vec<Product>,
}
#[derive(Clone, Serialize, JsonSchema)]
struct Product {
    sku: String,
    name: String,
    price_cents: u64,
}

/// Stateless tool: the macro generates the `Tool` implementation.
#[carmy::tool(
    description = "Search the product catalog",
    effect = "read",
    idempotent = true,
    parallel_safe = true
)]
async fn search_products(_ctx: AgentContext, input: SearchInput) -> AgentResult<SearchOutput> {
    let query = input.query.to_lowercase();
    let products = catalog()
        .into_iter()
        .filter(|p| p.name.to_lowercase().contains(&query))
        .collect();
    Ok(SearchOutput { products })
}
fn catalog() -> Vec<Product> {
    [
        ("KB-01", "Mechanical keyboard", 12_900),
        ("MS-02", "Wireless mouse", 4_900),
        ("MN-03", "4K monitor", 39_900),
    ]
    .into_iter()
    .map(|(sku, name, price_cents)| Product {
        sku: sku.into(),
        name: name.into(),
        price_cents,
    })
    .collect()
}

/// Application state is injected explicitly through the tool value,
/// never looked up from the framework context.
#[derive(Default)]
struct Orders(Mutex<BTreeMap<u64, String>>);

#[derive(Deserialize, JsonSchema)]
struct CreateOrderInput {
    sku: String,
}
#[derive(Serialize, JsonSchema)]
struct CreateOrderOutput {
    order_id: u64,
}
/// Stateful tool: implement `Tool` directly on a struct holding dependencies.
struct CreateOrder(Arc<Orders>);
impl Tool for CreateOrder {
    type Input = CreateOrderInput;
    type Output = CreateOrderOutput;
    fn metadata(&self) -> carmy::ToolMetadata {
        carmy::ToolMetadata {
            name: "create_order".into(),
            description: "Place an order for one product. Send a request_id to retry safely."
                .into(),
            input_schema: carmy::schema::<CreateOrderInput>(),
            output_schema: carmy::schema::<CreateOrderOutput>(),
            effect: Effect::Write,
            idempotent: false,
            parallel_safe: true,
            confirmation: Confirmation::None,
        }
    }
    async fn execute(
        &self,
        _ctx: AgentContext,
        input: CreateOrderInput,
    ) -> AgentResult<CreateOrderOutput> {
        if !catalog().iter().any(|p| p.sku == input.sku) {
            return Err(AgentError::new(
                "PRODUCT_NOT_FOUND",
                "No product has this SKU",
                ErrorCategory::NotFound,
            )
            .recoverable()
            .suggest("search_products"));
        }
        let mut orders = self.0.0.lock().unwrap();
        let order_id = orders.len() as u64 + 1;
        orders.insert(order_id, input.sku);
        Ok(CreateOrderOutput { order_id })
    }
}

#[derive(Deserialize, JsonSchema)]
struct CancelOrderInput {
    order_id: u64,
}
#[derive(Serialize, JsonSchema)]
struct CancelOrderOutput {
    cancelled: bool,
}
/// Destructive: the default policy rejects it unless the host grants
/// `confirm:cancel_order` in the trusted context.
struct CancelOrder(Arc<Orders>);
impl Tool for CancelOrder {
    type Input = CancelOrderInput;
    type Output = CancelOrderOutput;
    fn metadata(&self) -> carmy::ToolMetadata {
        carmy::ToolMetadata {
            name: "cancel_order".into(),
            description: "Cancel an order permanently".into(),
            input_schema: carmy::schema::<CancelOrderInput>(),
            output_schema: carmy::schema::<CancelOrderOutput>(),
            effect: Effect::Destructive,
            idempotent: true,
            parallel_safe: true,
            confirmation: Confirmation::Required,
        }
    }
    async fn execute(
        &self,
        _ctx: AgentContext,
        input: CancelOrderInput,
    ) -> AgentResult<CancelOrderOutput> {
        let cancelled = self.0.0.lock().unwrap().remove(&input.order_id).is_some();
        Ok(CancelOrderOutput { cancelled })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    carmy::observability::init()?;
    let orders = Arc::new(Orders::default());
    let server = Carmy::new()
        .name("tool-server")
        .tool(search_products)
        .tool(CreateOrder(orders.clone()))
        .tool(CancelOrder(orders));
    if std::env::args().any(|a| a == "--mcp") {
        server.serve_mcp_stdio().await?;
    } else {
        let address = std::env::var("CARMY_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".into());
        eprintln!("listening on http://{address}/.well-known/agent");
        server.listen(address).await?;
    }
    Ok(())
}
