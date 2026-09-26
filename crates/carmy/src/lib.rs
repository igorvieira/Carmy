//! Agent-native execution infrastructure for Rust.
//!
//! Carmy is a deterministic execution boundary between agents and real systems:
//! typed tools with explicit effects, machine-readable errors, idempotent retries,
//! cancellation and tracing, exposed over replaceable transports (HTTP, MCP).
//!
//! APIs are unstable during 0.x development.
//!
//! ```no_run
//! use carmy::prelude::*;
//!
//! #[derive(Deserialize, JsonSchema)]
//! struct SearchInput {
//!     /// Search expression.
//!     query: String,
//! }
//! #[derive(Serialize, JsonSchema)]
//! struct SearchOutput {
//!     results: Vec<String>,
//! }
//!
//! #[carmy::tool(description = "Search the catalog", effect = "read", idempotent = true)]
//! async fn search(_ctx: AgentContext, input: SearchInput) -> AgentResult<SearchOutput> {
//!     Ok(SearchOutput { results: vec![format!("Result for {}", input.query)] })
//! }
//!
//! # async fn run() -> Result<(), carmy::Error> {
//! Carmy::new().tool(search).listen("0.0.0.0:3000").await
//! # }
//! ```
pub use carmy_core::*;
pub use carmy_macros::tool;
pub use carmy_schema::schema;
pub use {schemars, serde, serde_json};
mod app;
mod config;
pub mod console;
mod state;
pub use app::{Carmy, DEFAULT_ADDRESS, Error, Result, app, run};
pub use config::Config;
pub mod testing;
pub use state::{IntoTool, State, StateMap};

#[doc(hidden)]
pub mod __private {
    pub use linkme;
    /// Registrations emitted by `#[carmy::tool]`, collected at link time.
    #[linkme::distributed_slice]
    pub static TOOLS: [fn(crate::Carmy) -> crate::Carmy];
}

/// Execution engine, policies and idempotency stores.
pub mod runtime {
    pub use carmy_runtime::*;
}
/// Tools that run later: queue, retries, dead letters and schedules.
pub mod jobs {
    pub use carmy_jobs::*;
}
/// HTTP transport: discovery, tool listing, execution and SSE.
#[cfg(feature = "http")]
pub mod http {
    pub use carmy_http::*;
}
/// MCP transport over the same runtime.
#[cfg(feature = "mcp")]
pub mod mcp {
    pub use carmy_mcp::*;
}
/// Tracing subscriber setup.
#[cfg(feature = "observability")]
pub mod observability {
    pub use carmy_observability::*;
}

pub mod prelude {
    pub use crate::{
        AgentContext, AgentError, AgentResult, Carmy, Confirmation, Effect, ErrorCategory, State,
        Tool,
    };
    pub use schemars::JsonSchema;
    pub use serde::{Deserialize, Serialize};
}
