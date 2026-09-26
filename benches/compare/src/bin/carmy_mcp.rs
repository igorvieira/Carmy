//! Carmy over MCP stdio, through the conventional entry point.
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
    // `run` reads the command (`mcp`, `server`) from argv, like any Carmy app.
    carmy::app().run().await
}
