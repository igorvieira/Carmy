//! Carmy over HTTP (`POST /agent/execute`), through the conventional entry point.
//! The address comes from CARMY_ADDR.
use carmy::prelude::*;
use carmy_compare::{SearchInput, SearchOutput, search};

#[carmy::tool(
    description = "Search the product catalog",
    effect = "read",
    idempotent = true,
    parallel_safe = true
)]
async fn search_products(input: SearchInput) -> AgentResult<SearchOutput> {
    Ok(search(&input.query))
}

#[tokio::main]
async fn main() -> carmy::Result {
    carmy::app().run().await
}
