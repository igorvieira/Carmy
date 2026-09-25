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
use std::{sync::Arc, time::Duration};
pub use {schemars, serde, serde_json};
mod state;
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

/// Failure to start a Carmy server.
#[derive(Debug)]
pub enum Error {
    /// A tool could not be registered (invalid or duplicate name, invalid schema).
    Registration(AgentError),
    Io(std::io::Error),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Registration(e) => write!(f, "tool registration failed: {e}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Registration(e) => Some(e),
            Self::Io(e) => Some(e),
        }
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Builder for a runtime and the transports that expose it.
///
/// Registration errors are collected and reported by [`Carmy::build`] or by the
/// transport entry points, so tool registration stays chainable.
pub struct Carmy {
    runtime: carmy_runtime::Runtime,
    name: String,
    states: StateMap,
    /// Registration waits for `build`, so `.state(..)` may follow `.tool(..)`.
    tools: Vec<Registration>,
}
type Registration = Box<dyn FnOnce(&mut carmy_runtime::Runtime, &StateMap) -> AgentResult<()>>;
impl Default for Carmy {
    fn default() -> Self {
        Self::new()
    }
}
impl Carmy {
    pub fn new() -> Self {
        Self {
            runtime: carmy_runtime::Runtime::new(),
            name: "carmy".into(),
            states: StateMap::default(),
            tools: Vec::new(),
        }
    }
    /// Server name advertised by discovery documents.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
    pub fn tool<T: IntoTool + 'static>(mut self, tool: T) -> Self {
        self.tools.push(Box::new(move |runtime, states| {
            runtime.register(tool.into_tool(states)?)
        }));
        self
    }
    /// Register an application dependency for tools taking `State<T>`.
    pub fn state<T: Clone + Send + Sync + 'static>(mut self, value: T) -> Self {
        self.states.insert(value);
        self
    }
    /// Add a host policy; the default policy (confirmation for destructive tools) stays.
    pub fn policy(mut self, policy: impl carmy_runtime::ExecutionPolicy + 'static) -> Self {
        self.runtime = self.runtime.policy(policy);
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.runtime = self.runtime.timeout(timeout);
        self
    }
    pub fn idempotency_store(mut self, store: Arc<dyn carmy_runtime::IdempotencyStore>) -> Self {
        self.runtime = self.runtime.idempotency_store(store);
        self
    }
    /// The shared runtime, for embedding or for serving several transports.
    pub fn build(mut self) -> Result<Arc<carmy_runtime::Runtime>, Error> {
        for register in self.tools {
            register(&mut self.runtime, &self.states).map_err(Error::Registration)?;
        }
        Ok(Arc::new(self.runtime))
    }
    #[cfg(feature = "http")]
    pub fn router(self) -> Result<axum::Router, Error> {
        let name = self.name.clone();
        Ok(carmy_http::router(self.build()?, name))
    }
    #[cfg(feature = "http")]
    pub async fn listen(self, address: impl tokio::net::ToSocketAddrs) -> Result<(), Error> {
        let name = self.name.clone();
        Ok(carmy_http::serve(self.build()?, address, name).await?)
    }
    #[cfg(feature = "mcp")]
    pub fn mcp(self) -> Result<carmy_mcp::McpServer, Error> {
        let name = self.name.clone();
        Ok(carmy_mcp::McpServer::new(self.build()?).name(name))
    }
    /// Serve one MCP client over stdin/stdout.
    #[cfg(feature = "mcp")]
    pub async fn serve_mcp_stdio(self) -> Result<(), Error> {
        Ok(self.mcp()?.serve_stdio().await?)
    }
}
