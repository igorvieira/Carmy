//! The real binary over MCP stdio: readiness-driven pipes, and the blocking fallback
//! when stdin is a regular file.
use rmcp::{ServiceExt, model::CallToolRequestParams};
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::{io::AsyncReadExt, process::Command};

const BIN: &str = env!("CARGO_BIN_EXE_tool-server");

#[tokio::test]
async fn serves_mcp_over_pipes() {
    let mut child = Command::new(BIN)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let transport = (child.stdout.take().unwrap(), child.stdin.take().unwrap());
    let client = ().serve(transport).await.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 3);
    // Concurrent calls exercise interleaved reads and writes on the pipes.
    let calls = (0..64).map(|_| {
        client.call_tool(
            CallToolRequestParams::new("search_products")
                .with_arguments(json!({"query": "mouse"}).as_object().unwrap().clone()),
        )
    });
    for result in futures_util::future::join_all(calls).await {
        let result = result.unwrap();
        assert_eq!(
            result.structured_content.unwrap()["products"][0]["sku"],
            "MS-02"
        );
    }
}

#[tokio::test]
async fn falls_back_when_stdin_is_a_file() {
    let dir = std::env::temp_dir().join(format!("carmy-stdio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("input.jsonl");
    let lines = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    ];
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(&input, text).unwrap();
    let mut child = Command::new(BIN)
        .arg("mcp")
        .stdin(std::fs::File::open(&input).unwrap())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut out = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        child.stdout.take().unwrap().read_to_string(&mut out),
    )
    .await
    .expect("server should exit at end of input")
    .unwrap();
    let responses: Vec<Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(responses.iter().any(|r| {
        r["id"] == 2
            && r["result"]["tools"]
                .as_array()
                .is_some_and(|t| t.len() == 3)
    }));
}
