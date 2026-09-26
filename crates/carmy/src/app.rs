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
    job_store: Arc<dyn carmy_jobs::JobStore>,
    retry: carmy_jobs::RetryPolicy,
    worker_concurrency: usize,
    schedules: Vec<ScheduleSpec>,
    #[cfg(feature = "http")]
    webhooks: Vec<(String, carmy_http::Webhook)>,
}
type ScheduleSpec = (
    String,
    String,
    Box<dyn Fn() -> crate::ExecutionRequest + Send + Sync>,
);
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
            job_store: Arc::new(carmy_jobs::InMemoryJobStore::default()),
            retry: carmy_jobs::RetryPolicy::default(),
            worker_concurrency: 4,
            schedules: Vec::new(),
            #[cfg(feature = "http")]
            webhooks: Vec::new(),
        }
    }
    /// Receive a provider's webhook at `path` as a tool execution; see
    /// [`carmy_http::Webhook`]. Hooks that `.enqueue()` use the app's job queue.
    #[cfg(feature = "http")]
    pub fn webhook(mut self, path: impl Into<String>, hook: carmy_http::Webhook) -> Self {
        self.webhooks.push((path.into(), hook));
        self
    }
    /// Where jobs wait. The default is process-local and in memory; use a durable
    /// store in production.
    pub fn jobs(mut self, store: Arc<dyn carmy_jobs::JobStore>) -> Self {
        self.job_store = store;
        self
    }
    /// Retry limits and backoff for jobs.
    pub fn retry(mut self, policy: carmy_jobs::RetryPolicy) -> Self {
        self.retry = policy;
        self
    }
    /// Enqueue `make()` on a cron schedule (seven fields, seconds first); see
    /// [`carmy_jobs::Jobs::every`].
    pub fn schedule(
        mut self,
        name: impl Into<String>,
        expression: impl Into<String>,
        make: impl Fn() -> crate::ExecutionRequest + Send + Sync + 'static,
    ) -> Self {
        self.schedules
            .push((name.into(), expression.into(), Box::new(make)));
        self
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
        if let Some(concurrency) = config.jobs.concurrency {
            self.worker_concurrency = concurrency.max(1);
        }
        if let Some(max_attempts) = config.jobs.max_attempts {
            self.retry.max_attempts = max_attempts.max(1);
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
    pub fn build(self) -> Result<Arc<carmy_runtime::Runtime>, Error> {
        self.build_with_jobs().map(|(runtime, _)| runtime)
    }
    /// The runtime and the job queue bound to it. Tools may take `State<Jobs>`.
    pub fn build_with_jobs(
        mut self,
    ) -> Result<(Arc<carmy_runtime::Runtime>, carmy_jobs::Jobs), Error> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let jobs = carmy_jobs::Jobs::unbound(self.job_store).with_retry(self.retry);
        self.states.insert(jobs.clone());
        for register in self.tools {
            register(&mut self.runtime, &self.states).map_err(Error::Registration)?;
        }
        let runtime = Arc::new(self.runtime);
        jobs.bind(runtime.clone());
        for (name, expression, make) in self.schedules {
            jobs.every(name, &expression, make)
                .map_err(Error::Registration)?;
        }
        Ok((runtime, jobs))
    }
    /// The HTTP router, with the per-request protections from [`Carmy::http`].
    #[cfg(feature = "http")]
    pub fn router(mut self) -> Result<axum::Router, Error> {
        let (name, options) = (self.name.clone(), self.http.clone());
        let webhooks = std::mem::take(&mut self.webhooks);
        let (runtime, jobs) = self.build_with_jobs()?;
        let mut router = carmy_http::router(runtime.clone(), name);
        if !webhooks.is_empty() {
            let queue: Arc<dyn carmy_http::Enqueue> = Arc::new(JobQueue(jobs));
            router = router.merge(
                carmy_http::webhook_router(runtime, webhooks, Some(queue))
                    .map_err(Error::Registration)?,
            );
        }
        Ok(carmy_http::harden(router, &options))
    }
    #[cfg(feature = "http")]
    pub async fn listen(self, address: impl tokio::net::ToSocketAddrs) -> Result<(), Error> {
        let options = self.http.clone();
        let router = self.router()?;
        let listener = tokio::net::TcpListener::bind(address).await?;
        Ok(carmy_http::serve_listener(listener, router, &options).await?)
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
            Some("worker") => {
                let concurrency = self.worker_concurrency;
                let (_, jobs) = self.build_with_jobs()?;
                let shutdown = crate::CancellationToken::new();
                tokio::spawn({
                    let shutdown = shutdown.clone();
                    async move {
                        wait_for_shutdown_signal().await;
                        eprintln!("carmy: shutting down, finishing running jobs");
                        shutdown.cancel();
                    }
                });
                eprintln!("carmy: worker running with concurrency {concurrency}");
                jobs.work(concurrency, shutdown).await;
                Ok(())
            }
            Some("tools") => {
                let catalog = serde_json::to_string_pretty(&self.build()?.tools())
                    .expect("metadata serializes");
                println!("{catalog}");
                Ok(())
            }
            Some(other) => Err(Error::Usage(format!(
                "unknown command `{other}`; expected `server`, `worker`, `mcp`, `console` or `tools`"
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

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = signal(SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
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

/// The app's job queue as the webhooks' queue.
#[cfg(feature = "http")]
struct JobQueue(carmy_jobs::Jobs);
#[cfg(feature = "http")]
impl carmy_http::Enqueue for JobQueue {
    fn enqueue<'a>(
        &'a self,
        request: crate::ExecutionRequest,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = AgentResult<String>> + Send + 'a>> {
        Box::pin(async move { self.0.enqueue(request).await.map(|id| id.0) })
    }
}
