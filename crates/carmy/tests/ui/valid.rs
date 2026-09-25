use carmy::prelude::*;
#[carmy::tool(effect = "none")]
async fn echo(_: AgentContext, input: String) -> AgentResult<String> { Ok(input) }
fn main() { let _ = echo.metadata(); }
