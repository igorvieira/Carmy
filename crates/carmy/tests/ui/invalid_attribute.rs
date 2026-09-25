use carmy::prelude::*;
#[carmy::tool(effect = "magic")]
async fn broken(_: AgentContext, input: String) -> AgentResult<String> { Ok(input) }
fn main() {}
