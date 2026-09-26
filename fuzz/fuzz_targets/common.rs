//! A small runtime with one tool, shared by the fuzz targets.
use carmy::prelude::*;
use std::sync::Arc;

#[derive(Deserialize, JsonSchema)]
pub struct Input {
    pub query: String,
    pub limit: Option<u32>,
}
#[derive(Serialize, JsonSchema)]
pub struct Output {
    pub echoed: String,
}

#[carmy::tool(description = "Echo", effect = "read", idempotent = true, register = false)]
async fn echo(input: Input) -> AgentResult<Output> {
    Ok(Output {
        echoed: input.query,
    })
}

pub fn runtime() -> Arc<carmy::runtime::Runtime> {
    Carmy::new().tool(echo).build().expect("a valid runtime")
}

pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime")
        .block_on(future)
}
