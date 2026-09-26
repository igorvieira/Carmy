use crate::{AgentError, AgentResult, Config, IntoTool, StateMap};
use std::{sync::Arc, time::Duration};

/// `Result` with Carmy's startup [`Error`]; `fn main() -> carmy::Result`.
pub type Result<T = (), E = Error> = std::result::Result<T, E>;

/// Failure to start a Carmy application.
#[derive(Debug)]
pub enum Error {
    /// A tool could not be registered (invalid or duplicate name, invalid schema,
    /// missing state).
    Registration(AgentError),
    /// `carmy.toml` or a `CARMY_*` environment variable is invalid.
    Config(String),
    /// Unknown command-line command.
    Usage(String),
    Io(std::io::Error),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Registration(e) => write!(f, "tool registration failed: {e}"),
            Self::Config(e) => write!(f, "invalid configuration: {e}"),
            Self::Usage(e) => write!(f, "{e}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Registration(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::Config(_) | Self::Usage(_) => None,
        }
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Default HTTP address of [`Carmy::run`].
pub const DEFAULT_ADDRESS: &str = "127.0.0.1:3000";

/// Builder for a runtime and the transports that expose it.
///
/// [`carmy::app()`](crate::app) is the conventional entry point; `Carmy::new()` is the
/// explicit one, with no configuration file and no auto-registered tools.
/// Errors are collected and reported by [`Carmy::build`] and the entry points, so the
/// builder stays chainable.
pub struct Carmy {
    runtime: carmy_runtime::Runtime,
    name: String,
    address: Option<String>,
    #[cfg(feature = "http")]
    http: carmy_http::ServerOptions,
    error: Option<Error>,
    pub(crate) states: StateMap,
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
            address: None,
            #[cfg(feature = "http")]
            http: carmy_http::ServerOptions::default(),
            error: None,
            states: StateMap::default(),
            tools: Vec::new(),
        }
    }
    /// Server name advertised by discovery documents.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
    /// HTTP address used by [`Carmy::run`].
    pub fn address(mut self, address: impl Into<String>) -> Self {
        self.address = Some(address.into());
        self
    }
    /// HTTP timeouts, connection limit, security headers and CORS.
    #[cfg(feature = "http")]
    pub fn http(mut self, options: carmy_http::ServerOptions) -> Self {
        self.http = options;
        self
    }
    /// Apply a loaded configuration.
    pub fn config(mut self, config: Config) -> Self {
        if let Some(name) = config.name {
            self.name = name;
        }
        if let Some(address) = config.address {
            self.address = Some(address);
        }
        if let Some(seconds) = config.timeout_secs {
            self = self.timeout(Duration::from_secs(seconds));
        }
        #[cfg(feature = "http")]
        {
            let http = &config.http;
            if let Some(seconds) = http.header_timeout_secs {
                self.http.header_timeout = Duration::from_secs(seconds);
            }
            if let Some(seconds) = http.body_timeout_secs {
                self.http.body_timeout = Duration::from_secs(seconds);
            }
            if let Some(max) = http.max_connections {
                self.http.max_connections = max;
            }
            if let Some(enabled) = http.security_headers {
                self.http.security_headers = enabled;
            }
        }
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
        if let Some(error) = self.error {
            return Err(error);
        }
        for register in self.tools {
            register(&mut self.runtime, &self.states).map_err(Error::Registration)?;
        }
        Ok(Arc::new(self.runtime))
    }
    /// The HTTP router, with the per-request protections from [`Carmy::http`].
    #[cfg(feature = "http")]
    pub fn router(self) -> Result<axum::Router, Error> {
        let (name, options) = (self.name.clone(), self.http.clone());
        Ok(carmy_http::router_with(self.build()?, name, &options))
    }
    #[cfg(feature = "http")]
    pub async fn listen(self, address: impl tokio::net::ToSocketAddrs) -> Result<(), Error> {
        let (name, options) = (self.name.clone(), self.http.clone());
        Ok(carmy_http::serve_with(self.build()?, address, name, &options).await?)
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
    /// Run the application according to the first command-line argument:
    ///
    /// | command            | effect                                            |
    /// |--------------------|---------------------------------------------------|
    /// | *(none)*, `server` | serve HTTP on the configured address              |
    /// | `mcp`              | serve MCP over stdin/stdout                       |
    /// | `console`          | serve the `carmy-console/1` protocol over stdio   |
    /// | `tools`            | print the tool catalog as JSON and exit           |
    pub async fn run(self) -> Result {
        let command = std::env::args().nth(1);
        // The console's stdout carries its protocol, so it logs warnings only by default.
        #[cfg(feature = "observability")]
        let _ = carmy_observability::init_with(match command.as_deref() {
            Some("console") => "warn",
            _ => "info",
        });
        match command.as_deref() {
            None | Some("server") => self.run_http().await,
            Some("console") => {
                let name = self.name.clone();
                let input = tokio::io::BufReader::new(tokio::io::stdin());
                Ok(crate::console::serve(self.build()?, &name, input, tokio::io::stdout()).await?)
            }
            Some("mcp") => self.run_mcp().await,
            Some("tools") => {
                let catalog = serde_json::to_string_pretty(&self.build()?.tools())
                    .expect("metadata serializes");
                println!("{catalog}");
                Ok(())
            }
            Some(other) => Err(Error::Usage(format!(
                "unknown command `{other}`; expected `server`, `mcp`, `console` or `tools`"
            ))),
        }
    }
    #[cfg(feature = "http")]
    async fn run_http(self) -> Result {
        let address = self
            .address
            .clone()
            .unwrap_or_else(|| DEFAULT_ADDRESS.into());
        let options = self.http.clone();
        let router = self.router()?;
        let listener = tokio::net::TcpListener::bind(&address).await?;
        eprintln!("carmy: serving HTTP on http://{address}/.well-known/agent");
        Ok(carmy_http::serve_listener(listener, router, &options).await?)
    }
    #[cfg(not(feature = "http"))]
    async fn run_http(self) -> Result {
        Err(Error::Usage("the `http` feature is disabled".into()))
    }
    #[cfg(feature = "mcp")]
    async fn run_mcp(self) -> Result {
        self.serve_mcp_stdio().await
    }
    #[cfg(not(feature = "mcp"))]
    async fn run_mcp(self) -> Result {
        Err(Error::Usage("the `mcp` feature is disabled".into()))
    }
}

/// The conventional application: configuration from `carmy.toml` and `CARMY_*`
/// environment variables, plus every tool declared with `#[carmy::tool]`
/// (unless `register = false`).
pub fn app() -> Carmy {
    let mut app = match Config::load() {
        Ok(config) => Carmy::new().config(config),
        Err(error) => Carmy {
            error: Some(error),
            ..Carmy::new()
        },
    };
    for register in crate::__private::TOOLS {
        app = register(app);
    }
    app
}

/// Shorthand for `carmy::app().run().await`.
pub async fn run() -> Result {
    app().run().await
}
