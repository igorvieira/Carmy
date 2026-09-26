//! Connection limits, timeouts, security headers and CORS for the HTTP adapter.
//!
//! The runtime's deadline starts when an execution starts. Everything before that, a
//! client trickling headers or a body, or holding an idle connection, is bounded here,
//! so a server can face the internet without a proxy in front.
use axum::{Router, http::HeaderValue};
use hyper_util::{
    rt::{TokioExecutor, TokioIo, TokioTimer},
    server::conn::auto,
    service::TowerToHyperService,
};
use std::{sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::Semaphore};
pub use tower_http::cors::{Any, CorsLayer};
use tower_http::{set_header::SetResponseHeaderLayer, timeout::RequestBodyTimeoutLayer};

/// Limits and protections applied by [`serve_with`](crate::serve_with) and
/// [`router_with`](crate::router_with). The defaults suit an internet-facing server.
#[derive(Debug, Clone)]
pub struct ServerOptions {
    /// Longest wait for a request's headers, and for the next request on an idle
    /// keep-alive connection. Bounds slowloris-style clients. Default: 10 seconds.
    pub header_timeout: Duration,
    /// Longest wait for a request body to arrive in full. Default: 30 seconds.
    pub body_timeout: Duration,
    /// Connections served at once; further ones wait in the accept backlog.
    /// Default: 4096.
    pub max_connections: usize,
    /// Add `X-Content-Type-Options`, `X-Frame-Options`, `Referrer-Policy` and a
    /// deny-all `Content-Security-Policy` to every response. Off by default: the
    /// API serves JSON, so these matter only when it is exposed to browsers.
    pub security_headers: bool,
    /// Cross-origin access for browsers. Off by default: no origin may call the API
    /// from a page. `CorsLayer::new().allow_origin(...)` narrows it to yours.
    pub cors: Option<CorsLayer>,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            header_timeout: Duration::from_secs(10),
            body_timeout: Duration::from_secs(30),
            max_connections: 4096,
            security_headers: false,
            cors: None,
        }
    }
}

/// Applies the per-request options (body timeout, security headers, CORS) to a router.
pub fn harden(router: Router, options: &ServerOptions) -> Router {
    let mut router = router.layer(RequestBodyTimeoutLayer::new(options.body_timeout));
    if options.security_headers {
        for (name, value) in [
            ("x-content-type-options", "nosniff"),
            ("x-frame-options", "DENY"),
            ("referrer-policy", "no-referrer"),
            (
                "content-security-policy",
                "default-src 'none'; frame-ancestors 'none'",
            ),
        ] {
            router = router.layer(SetResponseHeaderLayer::if_not_present(
                axum::http::HeaderName::from_static(name),
                HeaderValue::from_static(value),
            ));
        }
    }
    if let Some(cors) = &options.cors {
        router = router.layer(cors.clone());
    }
    router
}

/// Serves `router` on `listener` with the connection-level options: the header and
/// idle timeout, and the connection limit. The router should already be hardened.
pub async fn serve_listener(
    listener: TcpListener,
    router: Router,
    options: &ServerOptions,
) -> std::io::Result<()> {
    let permits = Arc::new(Semaphore::new(options.max_connections.max(1)));
    let mut builder = auto::Builder::new(TokioExecutor::new());
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(options.header_timeout);
    builder.http2().timer(TokioTimer::new());
    let builder = Arc::new(builder);
    loop {
        // Taking the permit first leaves excess connections in the kernel backlog
        // instead of accepting them into memory.
        let permit = permits
            .clone()
            .acquire_owned()
            .await
            .expect("the semaphore is never closed");
        let (stream, _) = match listener.accept().await {
            Ok(accepted) => accepted,
            // Transient errors (e.g. too many open files) must not stop the server.
            Err(e) if e.kind() != std::io::ErrorKind::InvalidInput => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
            Err(e) => return Err(e),
        };
        let service = TowerToHyperService::new(router.clone());
        let builder = builder.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let _ = builder
                .serve_connection_with_upgrades(TokioIo::new(stream), service)
                .await;
        });
    }
}
