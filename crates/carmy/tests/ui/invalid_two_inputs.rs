use carmy::prelude::*;
#[carmy::tool(effect = "read")]
async fn broken(a: String, b: String) -> AgentResult<String> { Ok(a + &b) }
fn main() {}
