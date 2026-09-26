//! A deal curator: offers come in from sources, become scored deals, and go out to
//! publishers, premium members first. The reference pipeline application.
//!
//! ```text
//! cargo run -p curator                  # HTTP: agent routes, /webhooks/billing, /deals, /ready
//! cargo run -p curator -- worker        # runs the jobs and the schedules
//! cargo run -p curator -- console       # try the tools; `audit`, `dead`
//! DATABASE_URL=postgres://.. cargo run -p curator --features postgres -- migrate
//! ```
#[tokio::main]
async fn main() -> carmy::Result {
    curator::app(curator::Settings::from_env()).run().await
}
