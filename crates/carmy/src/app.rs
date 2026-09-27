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
    audit: Arc<carmy_runtime::InMemoryAudit>,
    #[cfg(feature = "http")]
    webhooks: Vec<(String, carmy_http::Webhook)>,
    #[cfg(feature = "http")]
    readiness: Vec<(String, Arc<dyn carmy_http::ReadyCheck>)>,
    #[cfg(feature = "http")]
    routes: Vec<axum::Router>,
    worker_liveness: Option<Duration>,
    commands: std::collections::HashMap<String, Command>,
    operator_tools: bool,
    #[cfg(any(feature = "postgres", feature = "redis"))]
    database: Option<Database>,
    /// Where MCP is served over HTTP, and which `Host` values it accepts.
    #[cfg(all(feature = "http", feature = "mcp"))]
    mcp_http: Option<(String, Option<Vec<String>>)>,
    /// `[mcp]` settings applied to every MCP server this app builds.
    #[cfg(feature = "mcp")]
    mcp_settings: crate::config::McpConfig,
}
/// A configured database and how long `cleanup` keeps rows.
#[cfg(any(feature = "postgres", feature = "redis"))]
struct Database {
    backend: crate::database::Backend,
    retention: crate::Retention,
}
/// An app-defined command: receives the builder and runs to completion.
pub type Command =
    Box<dyn FnOnce(Carmy) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result>>>>;
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
            audit: Arc::new(carmy_runtime::InMemoryAudit::default()),
            #[cfg(feature = "http")]
            webhooks: Vec::new(),
            #[cfg(feature = "http")]
            readiness: Vec::new(),
            #[cfg(feature = "http")]
            routes: Vec::new(),
            worker_liveness: None,
            commands: std::collections::HashMap::new(),
            operator_tools: false,
            #[cfg(any(feature = "postgres", feature = "redis"))]
            database: None,
            #[cfg(all(feature = "http", feature = "mcp"))]
            mcp_http: Some(("/mcp".into(), None)),
            #[cfg(feature = "mcp")]
            mcp_settings: Default::default(),
        }
    }
    /// Serve MCP over Streamable HTTP at `path`, next to the agent routes and behind
    /// the same protections. On by default at `/mcp`. `allowed_hosts` lists the `Host`
    /// values accepted, a guard against DNS rebinding; `None` accepts localhost only,
    /// so list your public hosts in production.
    #[cfg(all(feature = "http", feature = "mcp"))]
    pub fn mcp_http(mut self, path: impl Into<String>, allowed_hosts: Option<Vec<String>>) -> Self {
        self.mcp_http = Some((path.into(), allowed_hosts));
        self
    }
    /// Serve MCP over stdio only.
    #[cfg(all(feature = "http", feature = "mcp"))]
    pub fn without_mcp_http(mut self) -> Self {
        self.mcp_http = None;
        self
    }
    /// Add `carmy_dead_letters` and `carmy_audit`, read-only tools that show the
    /// dead-letter queue and the latest executions to agents. They reveal who ran
    /// what, so pair them with a policy such as `RequireToolPermission`.
    /// (`carmy_job`, a job's status by id, is always present.)
    pub fn operator_tools(mut self) -> Self {
        self.operator_tools = true;
        self
    }
    /// Keep the app's durable state in the database at `url` (`[database] url` in
    /// `carmy.toml`, or `DATABASE_URL`); the scheme picks the store:
    ///
    /// - `postgres://` (feature `postgres`): jobs, idempotency and the audit trail.
    ///   Tools may take `State<PgPool>`, and `State<PostgresJobStore>` for the outbox.
    /// - `redis://` or `rediss://` (feature `redis`): jobs and idempotency; the audit
    ///   trail stays in memory. Tools may take `State<carmy::redis::Redis>`.
    ///
    /// Nothing connects until first use; `run()` migrates before any command but
    /// `tools`. Adds the `database` readiness check and the `migrate` and `cleanup`
    /// commands.
    #[cfg(any(feature = "postgres", feature = "redis"))]
    pub fn database(mut self, url: &str) -> Self {
        let retention = self
            .database
            .take()
            .map_or_else(crate::Retention::default, |d| d.retention);
        self.database_with(url, retention)
    }
    #[cfg(any(feature = "postgres", feature = "redis"))]
    fn database_with(mut self, url: &str, retention: crate::Retention) -> Self {
        use crate::database::Backend;
        let backend = match Backend::open(url) {
            Ok(backend) => backend,
            Err(message) => {
                self.error = Some(Error::Config(message));
                return self;
            }
        };
        match &backend {
            #[cfg(feature = "postgres")]
            Backend::Postgres(pool) => {
                let store = carmy_postgres::PostgresJobStore::new(pool.clone());
                self.job_store = Arc::new(store.clone());
                self.runtime = self
                    .runtime
                    .idempotency_store(Arc::new(carmy_postgres::PostgresIdempotencyStore::new(
                        pool.clone(),
                    )))
                    .sink(Arc::new(carmy_postgres::PostgresAudit::new(pool.clone())));
                self.states.insert(pool.clone());
                self.states.insert(store);
                #[cfg(feature = "http")]
                {
                    let check = pool.clone();
                    self = self.ready("database", move || carmy_postgres::ready(check.clone()));
                }
            }
            #[cfg(feature = "redis")]
            Backend::Redis(redis) => {
                let redis: &carmy_redis::Redis = redis;
                let store = carmy_redis::RedisJobStore::new(redis.clone());
                self.job_store = Arc::new(store.clone());
                self.runtime = self.runtime.idempotency_store(Arc::new(
                    carmy_redis::RedisIdempotencyStore::new(redis.clone())
                        .with_retention(retention.idempotency),
                ));
                self.states.insert(redis.clone());
                self.states.insert(store);
                #[cfg(feature = "http")]
                {
                    let check = redis.clone();
                    self = self.ready("database", move || carmy_redis::ready(check.clone()));
                }
            }
        }
        self.database = Some(Database { backend, retention });
        self
    }
    /// How long the `cleanup` command keeps finished work. Set it before
    /// [`Carmy::database`] for Redis, whose idempotency records expire on their own.
    #[cfg(any(feature = "postgres", feature = "redis"))]
    pub fn retention(mut self, retention: impl Into<crate::Retention>) -> Self {
        let retention = retention.into();
        match &mut self.database {
            Some(database) => database.retention = retention,
            None => {
                self.error.get_or_insert(Error::Config(
                    "retention needs a database; call .database(url) first".into(),
                ));
            }
        }
        self
    }
    /// A dependency `GET /ready` verifies; see [`carmy_http::ReadyCheck`]. A closure
    /// returning a future works: `.ready("database", move || ready(pool.clone()))`.
    #[cfg(feature = "http")]
    pub fn ready(
        mut self,
        name: impl Into<String>,
        check: impl carmy_http::ReadyCheck + 'static,
    ) -> Self {
        self.readiness.push((name.into(), Arc::new(check)));
        self
    }
    /// Report not ready unless a worker ticked the job queue within `within`. For
    /// servers that depend on their jobs actually running.
    pub fn require_worker(mut self, within: Duration) -> Self {
        self.worker_liveness = Some(within);
        self
    }
    /// Mount the app's own routes (a site, redirects, an admin) next to the agent
    /// routes, behind the same connection limits and timeouts.
    #[cfg(feature = "http")]
    pub fn routes(mut self, router: axum::Router) -> Self {
        self.routes.push(router);
        self
    }
    /// An extra command for [`Carmy::run`], e.g.
    /// `.command("seed", |app| async move { .. })`. The closure receives the builder,
    /// so it can build the runtime or read its state. It replaces a built-in command
    /// of the same name.
    pub fn command<F, Fut>(mut self, name: impl Into<String>, run: F) -> Self
    where
        F: FnOnce(Carmy) -> Fut + 'static,
        Fut: std::future::Future<Output = Result> + 'static,
    {
        self.commands
            .insert(name.into(), Box::new(move |app| Box::pin(run(app))));
        self
    }
    /// Also send every [`carmy_runtime::ExecutionRecord`] here (a durable audit trail,
    /// say). The in-memory trail the console shows stays on.
    pub fn sink(mut self, sink: Arc<dyn carmy_runtime::ExecutionSink>) -> Self {
        self.runtime = self.runtime.sink(sink);
        self
    }
    /// The last executions, newest first: what `carmy console`'s `audit` shows.
    pub fn audit(&self) -> Arc<carmy_runtime::InMemoryAudit> {
        self.audit.clone()
    }
    /// A door into a tool: deliveries to `path` are verified and run (or enqueued) as
    /// executions of the webhook's tool; see [`carmy_http::Webhook`].
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
        #[cfg(feature = "mcp")]
        {
            self.mcp_settings = config.mcp.clone();
        }
        #[cfg(all(feature = "http", feature = "mcp"))]
        {
            let mcp = &config.mcp;
            if mcp.http == Some(false) {
                self.mcp_http = None;
            } else if let Some((path, hosts)) = &mut self.mcp_http {
                if let Some(configured) = &mcp.path {
                    *path = configured.clone();
                }
                if mcp.allowed_hosts.is_some() {
                    *hosts = mcp.allowed_hosts.clone();
                }
            }
        }
        if let Some(url) = &config.database.url {
            #[cfg(any(feature = "postgres", feature = "redis"))]
            {
                // Retention first: Redis sets idempotency expiry when its store opens.
                let days = |d: u64| Duration::from_secs(d * 24 * 3600);
                let db = &config.database;
                let mut retention = crate::Retention::default();
                if let Some(d) = db.jobs_retention_days {
                    retention.jobs = days(d);
                }
                if let Some(d) = db.idempotency_retention_days {
                    retention.idempotency = days(d);
                }
                if let Some(d) = db.audit_retention_days {
                    retention.audit = days(d);
                }
                self = self.database_with(url, retention);
            }
            #[cfg(not(any(feature = "postgres", feature = "redis")))]
            {
                let _ = url;
                self.error = Some(Error::Config(
                    "database.url needs carmy's `postgres` or `redis` feature".into(),
                ));
            }
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
    /// Register several tools at once, e.g. the tools of an OpenAPI description.
    pub fn tools<T: IntoTool + 'static>(mut self, tools: impl IntoIterator<Item = T>) -> Self {
        for tool in tools {
            self = self.tool(tool);
        }
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
    /// The runtime and the job queue bound to it. Tools may take `State<Jobs>`. The
    /// queue does not keep the runtime alive: hold on to it while jobs run.
    pub fn build_with_jobs(
        mut self,
    ) -> Result<(Arc<carmy_runtime::Runtime>, carmy_jobs::Jobs), Error> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let store = self.job_store.clone();
        let jobs = carmy_jobs::Jobs::unbound(self.job_store).with_retry(self.retry);
        self.states.insert(jobs.clone());
        self.runtime = self.runtime.sink(self.audit.clone());
        // Registered first, so an app tool can never take their names silently.
        let system = self
            .runtime
            .register(crate::system::JobStatus(store.clone()))
            .and_then(|()| {
                if self.operator_tools {
                    self.runtime
                        .register(crate::system::DeadLettersTool(store.clone()))?;
                    self.runtime
                        .register(crate::system::AuditTool(self.audit.clone()))?;
                }
                Ok(())
            });
        system.map_err(Error::Registration)?;
        for register in self.tools {
            register(&mut self.runtime, &self.states).map_err(Error::Registration)?;
        }
        let runtime = Arc::new(self.runtime);
        // Weak: tools taking `State<Jobs>` hold the queue, so an owned bind would be a
        // cycle. Callers keep the runtime alive (router, console, MCP server, worker).
        jobs.bind_weak(&runtime);
        for (name, expression, make) in self.schedules {
            jobs.every(name, &expression, make)
                .map_err(Error::Registration)?;
        }
        Ok((runtime, jobs))
    }
    /// The HTTP router, with the per-request protections from [`Carmy::http`].
    #[cfg(feature = "http")]
    pub fn router(self) -> Result<axum::Router, Error> {
        self.router_and_jobs().map(|(router, _)| router)
    }
    /// [`Carmy::router`] plus the job queue behind it, for tests and custom workers.
    #[cfg(feature = "http")]
    pub fn router_and_jobs(mut self) -> Result<(axum::Router, carmy_jobs::Jobs), Error> {
        let (name, options) = (self.name.clone(), self.http.clone());
        let webhooks = std::mem::take(&mut self.webhooks);
        let mut readiness = std::mem::take(&mut self.readiness);
        let routes = std::mem::take(&mut self.routes);
        let worker_liveness = self.worker_liveness;
        #[cfg(feature = "mcp")]
        let mcp_http = self.mcp_http.take();
        #[cfg(feature = "mcp")]
        let settings = self.mcp_settings.clone();
        #[cfg(not(feature = "mcp"))]
        let mcp_http: Option<(String, Option<Vec<String>>)> = None;
        let (runtime, jobs) = self.build_with_jobs()?;
        if let Some(within) = worker_liveness {
            let jobs = jobs.clone();
            readiness.push((
                "worker".into(),
                Arc::new(move || {
                    let alive = jobs.worker_alive(within);
                    async move {
                        if alive {
                            Ok(())
                        } else {
                            Err(AgentError::new(
                                "WORKER_DOWN",
                                format!("no job worker ticked within {within:?}"),
                                crate::ErrorCategory::Capacity,
                            ))
                        }
                    }
                }),
            ));
        }
        let queue: Arc<dyn carmy_http::Enqueue> = Arc::new(JobQueue(jobs.clone()));
        let agent = carmy_http::AgentRoutes {
            webhooks,
            enqueuer: Some(queue),
            mcp_url: mcp_http.as_ref().map(|(path, _)| path.clone()),
        };
        let mut router = carmy_http::agent_routes(runtime.clone(), name.clone(), agent)
            .map_err(Error::Registration)?
            .merge(carmy_http::health_router(readiness));
        #[cfg(feature = "mcp")]
        if let Some((path, hosts)) = mcp_http {
            let service = mcp_server(runtime, name, &settings).http_service(hosts);
            router = router.nest_service(&path, service);
        }
        for own in routes {
            router = router.merge(own);
        }
        Ok((carmy_http::harden(router, &options), jobs))
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
        let (name, settings) = (self.name.clone(), self.mcp_settings.clone());
        Ok(mcp_server(self.build()?, name, &settings))
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
    /// | `worker`           | run the jobs and the schedules                    |
    /// | `migrate`          | create or update Carmy's tables (with a database) |
    /// | `cleanup`          | delete rows past their retention (with a database) |
    /// | *(custom)*         | anything added with [`Carmy::command`]             |
    pub async fn run(self) -> Result {
        let command = std::env::args().nth(1);
        self.run_command(command.as_deref()).await
    }
    /// [`Carmy::run`] with an explicit command instead of the process arguments.
    pub async fn run_command(mut self, command: Option<&str>) -> Result {
        // The console's stdout carries its protocol, so it logs warnings only by default.
        #[cfg(feature = "observability")]
        let _ = carmy_observability::init_with(match command {
            Some("console") => "warn",
            _ => "info",
        });
        #[cfg(any(feature = "postgres", feature = "redis"))]
        if command != Some("tools")
            && let Some(database) = &self.database
        {
            database
                .backend
                .migrate()
                .await
                .map_err(|e| Error::Io(std::io::Error::other(format!("database: {e}"))))?;
        }
        if let Some(run) = command.and_then(|c| self.commands.remove(c)) {
            return run(self).await;
        }
        #[cfg(any(feature = "postgres", feature = "redis"))]
        if let Some(database) = &self.database {
            match command {
                // Migrations already ran above.
                Some("migrate") => {
                    eprintln!("carmy: database is up to date");
                    return Ok(());
                }
                Some("cleanup") => {
                    let cleaned = database
                        .backend
                        .cleanup(database.retention)
                        .await
                        .map_err(|e| Error::Io(std::io::Error::other(e)))?;
                    println!("{cleaned}");
                    return Ok(());
                }
                _ => {}
            }
        }
        match command {
            None | Some("server") => self.run_http().await,
            Some("console") => {
                let name = self.name.clone();
                let audit = self.audit.clone();
                #[cfg(feature = "http")]
                let webhooks = self
                    .webhooks
                    .iter()
                    .map(|(path, hook)| hook.describe(path))
                    .collect();
                #[cfg(not(feature = "http"))]
                let webhooks = Vec::new();
                let input = tokio::io::BufReader::new(tokio::io::stdin());
                let (runtime, jobs) = self.build_with_jobs()?;
                let extras = crate::console::Extras {
                    audit: Some(audit),
                    jobs: Some(jobs),
                    webhooks,
                };
                Ok(
                    crate::console::serve_with(runtime, &name, extras, input, tokio::io::stdout())
                        .await?,
                )
            }
            Some("mcp") => self.run_mcp().await,
            Some("worker") => {
                let concurrency = self.worker_concurrency;
                // Held for as long as the worker runs: the queue only borrows it.
                let (_runtime, jobs) = self.build_with_jobs()?;
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
            Some(other) => {
                let mut known: Vec<&str> = vec!["server", "worker", "mcp", "console", "tools"];
                #[cfg(any(feature = "postgres", feature = "redis"))]
                if self.database.is_some() {
                    known.extend(["migrate", "cleanup"]);
                }
                known.extend(self.commands.keys().map(String::as_str));
                Err(Error::Usage(format!(
                    "unknown command `{other}`; expected one of: {}",
                    known.join(", ")
                )))
            }
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

/// An MCP server over `runtime`, with the app's `[mcp]` settings.
#[cfg(feature = "mcp")]
fn mcp_server(
    runtime: Arc<carmy_runtime::Runtime>,
    name: String,
    settings: &crate::config::McpConfig,
) -> carmy_mcp::McpServer {
    let mut server = carmy_mcp::McpServer::new(runtime).name(name);
    if let Some(ms) = settings.promote_after_ms {
        server = server.promote_after((ms > 0).then(|| Duration::from_millis(ms)));
    }
    if let Some(enabled) = settings.confirm_by_elicitation {
        server = server.confirm_by_elicitation(enabled);
    }
    server
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
