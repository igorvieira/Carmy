use carmy::prelude::*;
#[derive(Deserialize)]
struct Input;
#[carmy::tool(effect = "none")]
async fn broken(_: AgentContext, _: Input) -> AgentResult<String> { Ok(String::new()) }
fn main() {}
