//! Fixture tools shared by the benchmarks.
use carmy::prelude::*;

#[derive(Deserialize, JsonSchema)]
pub struct LookupInput {
    /// Product identifier.
    pub sku: String,
    pub quantity: u32,
}
#[derive(Serialize, JsonSchema)]
pub struct LookupOutput {
    pub sku: String,
    pub available: bool,
    pub price_cents: u64,
}

/// A read-only tool that does no work, so measurements show framework cost.
#[carmy::tool(
    description = "Look up product availability",
    effect = "read",
    idempotent = true,
    parallel_safe = true
)]
pub async fn lookup(_ctx: AgentContext, input: LookupInput) -> AgentResult<LookupOutput> {
    Ok(LookupOutput {
        sku: input.sku,
        available: input.quantity < 10,
        price_cents: 1_299,
    })
}
