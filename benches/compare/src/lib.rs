//! The workload every contender implements: `search_products` over a fixed in-memory
//! catalog. It is deliberately cheap, so the measurements show framework cost.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchInput {
    /// Case-insensitive substring of the product name.
    pub query: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Product {
    pub sku: String,
    pub name: String,
    pub price_cents: u64,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchOutput {
    pub products: Vec<Product>,
}

pub const PRODUCTS: &[(&str, &str, u64)] = &[
    ("KB-01", "Mechanical keyboard", 12_900),
    ("KB-02", "Low-profile keyboard", 9_900),
    ("MS-01", "Wireless mouse", 4_900),
    ("MS-02", "Vertical mouse", 5_900),
    ("MS-03", "Trackball mouse", 7_900),
    ("MN-01", "4K monitor", 39_900),
    ("MN-02", "Ultrawide monitor", 54_900),
    ("HD-01", "USB-C hub", 3_900),
    ("HP-01", "Noise-cancelling headphones", 24_900),
    ("WC-01", "1080p webcam", 6_900),
    ("MC-01", "USB microphone", 11_900),
    ("DS-01", "Standing desk", 49_900),
    ("CH-01", "Ergonomic chair", 39_900),
    ("LP-01", "Desk lamp", 2_900),
    ("PD-01", "Mouse pad", 1_900),
    ("CB-01", "Thunderbolt cable", 2_400),
];

pub fn search(query: &str) -> SearchOutput {
    let query = query.to_lowercase();
    SearchOutput {
        products: PRODUCTS
            .iter()
            .filter(|(_, name, _)| name.to_lowercase().contains(&query))
            .map(|&(sku, name, price_cents)| Product {
                sku: sku.into(),
                name: name.into(),
                price_cents,
            })
            .collect(),
    }
}

/// The query every benchmark sends, and how many products it must return.
pub const QUERY: &str = "mouse";
pub const EXPECTED_MATCHES: usize = 4;
