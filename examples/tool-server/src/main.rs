//! One set of tools, two transports, conventional style.
//!
//! ```text
//! cargo run -p tool-server             # HTTP on 127.0.0.1:3000 (CARMY_ADDR to change)
//! cargo run -p tool-server -- mcp      # MCP over stdio
//! cargo run -p tool-server -- tools    # print the catalog
//! ```
use carmy::prelude::*;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Serialize, JsonSchema)]
struct Product {
    sku: String,
    name: String,
    price_cents: u64,
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

#[derive(Deserialize, JsonSchema)]
struct SearchInput {
    /// Search expression, matched case-insensitively against product names.
    query: String,
}
#[derive(Serialize, JsonSchema)]
struct SearchOutput {
    products: Vec<Product>,
}

#[carmy::tool(
    description = "Search the product catalog",
    effect = "read",
    idempotent = true,
    parallel_safe = true
)]
async fn search_products(input: SearchInput) -> AgentResult<SearchOutput> {
    let query = input.query.to_lowercase();
    let products = catalog()
        .into_iter()
        .filter(|p| p.name.to_lowercase().contains(&query))
        .collect();
    Ok(SearchOutput { products })
}

/// Application state, registered once with `.state(..)` and injected with `State<T>`.
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

#[carmy::tool(
    description = "Place an order for one product. Send a request_id to retry safely.",
    effect = "write",
    parallel_safe = true
)]
async fn create_order(
    State(orders): State<Arc<Orders>>,
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
    let mut orders = orders.0.lock().unwrap();
    let order_id = orders.len() as u64 + 1;
    orders.insert(order_id, input.sku);
    Ok(CreateOrderOutput { order_id })
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
#[carmy::tool(
    description = "Cancel an order permanently",
    effect = "destructive",
    idempotent = true,
    parallel_safe = true,
    confirmation = "required"
)]
async fn cancel_order(
    State(orders): State<Arc<Orders>>,
    input: CancelOrderInput,
) -> AgentResult<CancelOrderOutput> {
    let cancelled = orders.0.lock().unwrap().remove(&input.order_id).is_some();
    Ok(CancelOrderOutput { cancelled })
}

#[tokio::main]
async fn main() -> carmy::Result {
    carmy::app()
        .name("tool-server")
        .state(Arc::new(Orders::default()))
        .run()
        .await
}
