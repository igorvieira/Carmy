//! Carmy over MCP stdio with execution metadata on every successful result.
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
    Ok(carmy::app()
        .mcp()?
        .execution_meta(true)
        .serve_stdio()
        .await?)
}
