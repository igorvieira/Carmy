//! The same tool on the official Rust MCP SDK alone, without Carmy.
use carmy_compare::{SearchInput, SearchOutput, search};
use rmcp::{
    ServiceExt,
    handler::server::wrapper::{Json, Parameters},
    tool, tool_router,
};

#[derive(Clone)]
struct Server;

#[tool_router(server_handler)]
impl Server {
    #[tool(description = "Search the product catalog")]
    async fn search_products(
        &self,
        Parameters(input): Parameters<SearchInput>,
    ) -> Json<SearchOutput> {
        Json(search(&input.query))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let running = Server.serve(rmcp::transport::stdio()).await?;
    running.waiting().await?;
    Ok(())
}
