use carmy::prelude::*;
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};

static CREATED: AtomicUsize = AtomicUsize::new(0);

#[derive(Deserialize, JsonSchema)]
struct Order {
    sku: String,
}
#[derive(Serialize, JsonSchema)]
struct Created {
    order_id: usize,
    sku: String,
}
#[carmy::tool(description = "Place an order", effect = "write", register = false)]
async fn create_order(input: Order) -> AgentResult<Created> {
    Ok(Created {
        order_id: CREATED.fetch_add(1, Ordering::SeqCst) + 1,
        sku: input.sku,
    })
}
#[carmy::tool(
    description = "Delete everything",
    effect = "destructive",
    confirmation = "required",
    register = false
)]
async fn wipe() -> AgentResult<bool> {
    Ok(true)
}

/// Runs a console session over the given lines and returns every output line as JSON.
async fn session(lines: &[&str]) -> Vec<Value> {
    let runtime = Carmy::new().tool(create_order).tool(wipe).build().unwrap();
    let input = lines.join("\n") + "\n";
    let mut output = Vec::new();
    carmy::console::serve(runtime, "test", input.as_bytes(), &mut output)
        .await
        .unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).expect("every output line is JSON"))
        .collect()
}

#[tokio::test]
async fn discovery_and_calls() {
    let out = session(&[
        r#"{"id":1,"op":"tools"}"#,
        r#"{"id":2,"op":"describe","tool":"create_order"}"#,
        r#"{"id":3,"op":"call","tool":"create_order","arguments":{"sku":"KB-01"}}"#,
        r#"{"id":4,"op":"call","tool":"create_order","arguments":{"sku":4}}"#,
    ])
    .await;
    assert_eq!(out[0]["event"], "ready");
    assert_eq!(out[0]["protocol"], "carmy-console/1");
    assert_eq!(out[0]["tools"], 2);
    assert_eq!(out[1]["id"], 1);
    assert_eq!(out[1]["result"]["tools"].as_array().unwrap().len(), 2);
    assert_eq!(out[2]["result"]["effect"], "write");
    let call = &out[3]["result"];
    assert_eq!(out[3]["ok"], true);
    assert_eq!(call["status"], "completed");
    assert_eq!(call["data"]["sku"], "KB-01");
    assert_eq!(call["replayed"], false);
    assert!(call["duration_ms"].is_number());
    // A failed execution is still a successful protocol exchange.
    assert_eq!(out[4]["ok"], true);
    assert_eq!(out[4]["result"]["error"]["code"], "INVALID_ARGUMENTS");
}

#[tokio::test]
async fn protocol_errors_are_structured() {
    let out = session(&[
        r#"{"id":1,"op":"describe","tool":"missing"}"#,
        r#"{"id":2,"op":"dance"}"#,
        r#"{"id":3}"#,
        r#"{"op": broken"#,
    ])
    .await;
    assert_eq!(out[1]["error"]["code"], "TOOL_NOT_FOUND");
    assert_eq!(out[1]["error"]["suggested_action"], "tools");
    assert_eq!(out[2]["error"]["code"], "UNKNOWN_OP");
    assert_eq!(out[3]["error"]["code"], "INVALID_REQUEST");
    assert_eq!(out[4]["ok"], false);
    assert_eq!(out[4]["error"]["code"], "INVALID_REQUEST");
    assert!(out[4].get("id").is_none());
}

#[tokio::test]
async fn request_ids_replay_and_confirmation_is_per_session() {
    let out = session(&[
        r#"{"op":"call","tool":"create_order","arguments":{"sku":"A"},"request_id":"r1"}"#,
        r#"{"op":"call","tool":"create_order","arguments":{"sku":"A"},"request_id":"r1"}"#,
        r#"{"op":"call","tool":"wipe"}"#,
        r#"{"op":"confirm","tool":"wipe"}"#,
        r#"{"op":"call","tool":"wipe"}"#,
        r#"{"op":"revoke","tool":"wipe"}"#,
        r#"{"op":"call","tool":"wipe"}"#,
    ])
    .await;
    assert_eq!(out[1]["result"]["replayed"], false);
    assert_eq!(out[2]["result"]["replayed"], true);
    // Every execution takes a new order_id, so an equal one proves the tool ran once.
    assert_eq!(out[1]["result"]["data"], out[2]["result"]["data"]);
    assert_eq!(out[3]["result"]["error"]["code"], "CONFIRMATION_REQUIRED");
    assert_eq!(out[4]["result"]["confirmed"], true);
    assert_eq!(out[5]["result"]["status"], "completed");
    assert_eq!(out[7]["result"]["error"]["code"], "CONFIRMATION_REQUIRED");
}

#[tokio::test]
async fn text_commands_answer_in_json() {
    let out = session(&[
        "tools",
        "describe wipe",
        r#"create_order {"sku": "KB-01"} --request-id t1"#,
        "create_order not json",
        "help",
        "exit",
        "tools",
    ])
    .await;
    assert_eq!(out[1]["result"]["tools"].as_array().unwrap().len(), 2);
    assert_eq!(out[2]["result"]["confirmation"], "required");
    assert_eq!(out[3]["result"]["data"]["sku"], "KB-01");
    assert_eq!(out[4]["error"]["code"], "INVALID_REQUEST");
    assert_eq!(out[5]["result"]["protocol"], "carmy-console/1");
    assert_eq!(out.len(), 6, "exit ends the session");
}
