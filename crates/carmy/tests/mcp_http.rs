//! MCP over Streamable HTTP, served by the app's own router on a real socket.
use axum::{body::Body, http::Request};
use carmy::prelude::*;
use http_body_util::BodyExt;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, ProgressNotificationParam},
    service::{NotificationContext, RoleClient},
    transport::StreamableHttpClientTransport,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tower::ServiceExt as _;

#[derive(Deserialize, JsonSchema)]
struct Pages {
    count: u32,
}
#[carmy::tool(
    description = "Import pages",
    effect = "read",
    idempotent = true,
    register = false
)]
async fn import_pages(ctx: AgentContext, input: Pages) -> AgentResult<u32> {
    for page in 1..=input.count {
        ctx.progress.report(
            page as f64,
            Some(input.count as f64),
            format!("page {page}"),
        );
    }
    Ok(input.count)
}

#[derive(Clone, Default)]
struct Watcher(Arc<Mutex<Vec<ProgressNotificationParam>>>);
impl rmcp::ClientHandler for Watcher {
    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _: NotificationContext<RoleClient>,
    ) {
        self.0.lock().unwrap().push(params);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_reach_the_tools_over_mcp_http() {
    let router = Carmy::new()
        .name("shop")
        .tool(import_pages)
        .router()
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let served = router.clone();
    let server = tokio::spawn(async move {
        let options = carmy::http::ServerOptions::default();
        carmy::http::serve_listener(listener, served, &options).await
    });

    // Discovery points agents at the endpoint.
    let discovery = router
        .clone()
        .oneshot(
            Request::get("/.well-known/agent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let discovery: Value =
        serde_json::from_slice(&discovery.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(discovery["mcp_url"], "/mcp");
    assert!(
        discovery["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("mcp"))
    );

    let watcher = Watcher::default();
    let transport = StreamableHttpClientTransport::from_uri(format!("http://{address}/mcp"));
    let client = watcher.clone().serve(transport).await.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    assert!(tools.iter().any(|t| t.name == "import_pages"));
    assert!(tools.iter().any(|t| t.name == "carmy_job"));

    let result = client
        .call_tool(
            CallToolRequestParams::new("import_pages")
                .with_arguments(json!({"count": 3}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(false));
    for _ in 0..50 {
        if watcher.0.lock().unwrap().len() == 3 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let seen = watcher.0.lock().unwrap().clone();
    assert_eq!(seen.len(), 3, "progress reaches the client over HTTP");
    assert_eq!(seen[2].message.as_deref(), Some("page 3"));

    client.cancel().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn unknown_hosts_are_refused_and_mcp_http_can_be_turned_off() {
    let router = Carmy::new().tool(import_pages).router().unwrap();
    let initialize = || {
        Request::post("/mcp")
            .header("host", "evil.example")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(Body::from(
                json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                    "protocolVersion": "2025-06-18", "capabilities": {},
                    "clientInfo": {"name": "t", "version": "1"}}})
                .to_string(),
            ))
            .unwrap()
    };
    let refused = router.clone().oneshot(initialize()).await.unwrap();
    assert_eq!(refused.status(), 403, "DNS rebinding guard");

    // Listed hosts are accepted.
    let allowed = Carmy::new()
        .tool(import_pages)
        .mcp_http("/mcp", Some(vec!["evil.example".into()]))
        .router()
        .unwrap();
    assert_eq!(allowed.oneshot(initialize()).await.unwrap().status(), 200);

    let off = Carmy::new()
        .tool(import_pages)
        .without_mcp_http()
        .router()
        .unwrap();
    assert_eq!(
        off.clone().oneshot(initialize()).await.unwrap().status(),
        404
    );
    let discovery = off
        .oneshot(
            Request::get("/.well-known/agent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let discovery: Value =
        serde_json::from_slice(&discovery.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(discovery.get("mcp_url").is_none());
}

#[carmy::tool(
    description = "Wipe everything",
    effect = "destructive",
    confirmation = "required",
    register = false
)]
async fn wipe() -> AgentResult<bool> {
    Ok(true)
}

async fn call_wipe(router: axum::Router) -> rmcp::model::CallToolResult {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let options = carmy::http::ServerOptions::default();
        carmy::http::serve_listener(listener, router, &options).await
    });
    let transport = StreamableHttpClientTransport::from_uri(format!("http://{address}/mcp"));
    let client = ().serve(transport).await.unwrap();
    let result = client
        .call_tool(CallToolRequestParams::new("wipe"))
        .await
        .unwrap();
    client.cancel().await.unwrap();
    result
}

#[tokio::test(flavor = "multi_thread")]
async fn the_hosts_authentication_reaches_mcp_calls() {
    // Without a grant, the destructive tool asks for confirmation.
    let plain = Carmy::new().tool(wipe).router().unwrap();
    let denied = call_wipe(plain).await;
    assert_eq!(denied.is_error, Some(true));

    // A host middleware that authenticated the caller and recorded the grant.
    let context = AgentContext {
        principal: Some("ada".into()),
        permissions: ["confirm:wipe".to_string()].into(),
        ..AgentContext::default()
    };
    let authenticated = Carmy::new()
        .tool(wipe)
        .router()
        .unwrap()
        .layer(axum::Extension(context));
    let allowed = call_wipe(authenticated).await;
    assert_eq!(allowed.is_error, Some(false), "{allowed:?}");
}
