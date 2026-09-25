//! Tracing setup for Carmy executions.
//!
//! The runtime emits one `carmy.execution` span per execution with the fields in
//! [`fields`], plus a completion event (`INFO` on success, `WARN` on failure). Tool
//! arguments and outputs are never recorded.
//!
//! Basic use calls [`init`]. OpenTelemetry needs no Carmy support: compose
//! [`fmt_layer`] with a `tracing-opentelemetry` layer in your own registry, and the
//! execution spans are exported with the same fields.
//!
//! ```no_run
//! use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
//! tracing_subscriber::registry()
//!     .with(EnvFilter::new("info"))
//!     .with(carmy_observability::fmt_layer())
//!     // .with(tracing_opentelemetry::layer().with_tracer(tracer))
//!     .init();
//! ```
use tracing_subscriber::{EnvFilter, Layer, Registry, layer::SubscriberExt};

/// Name of the span wrapping every execution.
pub const EXECUTION_SPAN: &str = "carmy.execution";
/// Fields recorded on [`EXECUTION_SPAN`].
pub mod fields {
    pub const EXECUTION_ID: &str = "execution_id";
    pub const REQUEST_ID: &str = "request_id";
    pub const TOOL: &str = "tool";
    pub const EFFECT: &str = "effect";
    pub const STATUS: &str = "status";
    pub const DURATION_MS: &str = "duration_ms";
    pub const REPLAYED: &str = "replayed";
    pub const ERROR_CODE: &str = "error_code";
}

/// Human-readable log lines on stderr (stdout is reserved for protocols like MCP stdio).
pub fn fmt_layer<S>() -> impl Layer<S> + Send + Sync
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
}

/// Install a global subscriber filtered by `RUST_LOG` (default `info`).
/// Returns an error if a global subscriber is already set.
pub fn init() -> Result<(), tracing::subscriber::SetGlobalDefaultError> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing::subscriber::set_global_default(Registry::default().with(filter).with(fmt_layer()))
}
