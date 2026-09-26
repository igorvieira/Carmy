//! Connection-level protections, exercised over real TCP sockets.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use carmy_core::*;
use carmy_http::{Any, CorsLayer, ServerOptions, router_with, serve_listener};
use carmy_runtime::Runtime;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};
use tower::ServiceExt;

struct Echo;
impl Tool for Echo {
    type Input = String;
    type Output = String;
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: "echo".into(),
            description: "echo".into(),
            input_schema: schemars::schema_for!(String).to_value(),
            output_schema: schemars::schema_for!(String).to_value(),
            effect: Effect::Read,
            idempotent: true,
            parallel_safe: true,
            confirmation: Confirmation::None,
        }
    }
    async fn execute(&self, _: AgentContext, input: String) -> AgentResult<String> {
        Ok(input)
    }
}

fn runtime() -> Arc<Runtime> {
    Arc::new(Runtime::new().tool(Echo).unwrap())
}

async fn spawn(options: ServerOptions) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = router_with(runtime(), "test", &options);
    tokio::spawn(async move { serve_listener(listener, router, &options).await });
    address
}

fn request() -> Vec<u8> {
    let body = r#"{"tool":"echo","arguments":"hi"}"#;
    format!(
        "POST /agent/execute HTTP/1.1\r\nHost: t\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Reads until the server closes the connection or `limit` passes.
async fn read_until_close(stream: &mut TcpStream, limit: Duration) -> Result<Vec<u8>, ()> {
    let mut all = Vec::new();
    let mut buf = [0u8; 4096];
    timeout(limit, async {
        loop {
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => all.extend_from_slice(&buf[..n]),
            }
        }
    })
    .await
    .map(|_| all)
    .map_err(|_| ())
}

#[tokio::test]
async fn slow_headers_are_cut_off() {
    let address = spawn(ServerOptions {
        header_timeout: Duration::from_millis(300),
        ..ServerOptions::default()
    })
    .await;
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream
        .write_all(b"POST /agent/execute HTTP/1.1\r\nHost: t\r\n")
        .await
        .unwrap();
    let start = std::time::Instant::now();
    let closed = read_until_close(&mut stream, Duration::from_secs(3)).await;
    assert!(
        closed.is_ok(),
        "a client that never finishes its headers must be dropped"
    );
    assert!(
        start.elapsed() >= Duration::from_millis(250),
        "closed too early"
    );
}

#[tokio::test]
async fn idle_keep_alive_connections_are_closed() {
    let address = spawn(ServerOptions {
        header_timeout: Duration::from_millis(300),
        ..ServerOptions::default()
    })
    .await;
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream.write_all(&request()).await.unwrap();
    let mut buf = [0u8; 4096];
    let n = timeout(Duration::from_secs(2), stream.read(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert!(
        std::str::from_utf8(&buf[..n])
            .unwrap()
            .starts_with("HTTP/1.1 200")
    );
    // Nothing more is sent: the idle connection must be closed by the server.
    let closed = read_until_close(&mut stream, Duration::from_secs(3)).await;
    assert!(
        closed.is_ok(),
        "an idle keep-alive connection must be closed"
    );
}

#[tokio::test]
async fn slow_bodies_are_cut_off() {
    let address = spawn(ServerOptions {
        body_timeout: Duration::from_millis(300),
        ..ServerOptions::default()
    })
    .await;
    let mut stream = TcpStream::connect(address).await.unwrap();
    // Complete headers, then a body that never comes.
    stream
        .write_all(b"POST /agent/execute HTTP/1.1\r\nHost: t\r\nContent-Type: application/json\r\nContent-Length: 50\r\n\r\n{\"tool\"")
        .await
        .unwrap();
    let output = read_until_close(&mut stream, Duration::from_secs(3))
        .await
        .expect("the request must be answered or dropped, not held");
    let text = String::from_utf8_lossy(&output);
    assert!(
        text.is_empty() || text.starts_with("HTTP/1.1 4"),
        "expected a client error or a closed socket, got: {text}"
    );
}

#[tokio::test]
async fn connection_limit_queues_extra_connections() {
    let address = spawn(ServerOptions {
        max_connections: 1,
        header_timeout: Duration::from_secs(10),
        ..ServerOptions::default()
    })
    .await;
    // The first connection is served and held open by sending nothing else.
    let mut first = TcpStream::connect(address).await.unwrap();
    first
        .write_all(b"GET /agent/tools HTTP/1.1\r\nHost: t\r\n")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    // The second one is accepted by the kernel but not served until the first ends.
    let mut second = TcpStream::connect(address).await.unwrap();
    second.write_all(&request()).await.unwrap();
    let mut buf = [0u8; 4096];
    let early = timeout(Duration::from_millis(500), second.read(&mut buf)).await;
    assert!(
        early.is_err(),
        "the second connection must wait for a free slot"
    );
    drop(first);
    let n = timeout(Duration::from_secs(3), second.read(&mut buf))
        .await
        .expect("served once a slot frees up")
        .unwrap();
    assert!(
        std::str::from_utf8(&buf[..n])
            .unwrap()
            .starts_with("HTTP/1.1 200")
    );
}

fn get(path: &str) -> Request<Body> {
    Request::get(path).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn security_headers_are_opt_in() {
    let plain = router_with(runtime(), "test", &ServerOptions::default());
    let response = plain.oneshot(get("/agent/tools")).await.unwrap();
    assert!(response.headers().get("x-frame-options").is_none());

    let hardened = router_with(
        runtime(),
        "test",
        &ServerOptions {
            security_headers: true,
            ..ServerOptions::default()
        },
    );
    for path in ["/agent/tools", "/nowhere"] {
        let response = hardened.clone().oneshot(get(path)).await.unwrap();
        let headers = response.headers();
        assert_eq!(headers["x-content-type-options"], "nosniff", "{path}");
        assert_eq!(headers["x-frame-options"], "DENY", "{path}");
        assert_eq!(headers["referrer-policy"], "no-referrer", "{path}");
        assert!(
            headers["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("default-src 'none'")
        );
    }
}

#[tokio::test]
async fn cors_is_opt_in() {
    let preflight = || {
        Request::options("/agent/execute")
            .header("origin", "https://app.example")
            .header("access-control-request-method", "POST")
            .body(Body::empty())
            .unwrap()
    };
    let plain = router_with(runtime(), "test", &ServerOptions::default());
    let response = plain.oneshot(preflight()).await.unwrap();
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );

    let open = router_with(
        runtime(),
        "test",
        &ServerOptions {
            cors: Some(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any),
            ),
            ..ServerOptions::default()
        },
    );
    let response = open.oneshot(preflight()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["access-control-allow-origin"], "*");
}
