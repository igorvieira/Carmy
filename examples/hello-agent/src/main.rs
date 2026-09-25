//! Drive the runtime in-process, without any transport.
//!
//! ```text
//! cargo run -p hello-agent
//! ```
use carmy::{prelude::*, runtime::execution_request};
use futures_util::StreamExt;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Deserialize, JsonSchema)]
struct CreateNote {
    /// Note body.
    text: String,
}
#[derive(Serialize, JsonSchema)]
struct Note {
    id: u64,
    text: String,
}

#[carmy::tool(description = "Store a note", effect = "write")]
async fn create_note(_ctx: AgentContext, input: CreateNote) -> AgentResult<Note> {
    if input.text.trim().is_empty() {
        return Err(AgentError::new(
            "EMPTY_NOTE",
            "Note text is empty",
            ErrorCategory::Validation,
        )
        .recoverable());
    }
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    Ok(Note {
        id,
        text: input.text,
    })
}

#[tokio::main]
async fn main() -> Result<(), carmy::Error> {
    let runtime = Carmy::new().tool(create_note).build()?;
    let describe = |label: &str, result: carmy::ExecutionResult| {
        let outcome = match result.outcome {
            Ok(data) => json!({ "data": data }),
            Err(error) => json!({ "error": error }),
        };
        println!("{label}: {} {outcome}", result.status.as_str());
    };

    // Discovery: metadata is data an agent can read without documentation.
    let metadata = &runtime.tools()[0];
    println!(
        "tool {} effect={} idempotent={}",
        metadata.name,
        metadata.effect.as_str(),
        metadata.idempotent
    );

    // A retried write with the same request_id executes once.
    let mut request = execution_request("create_note", json!({ "text": "hello" }));
    request.request_id = Some("note-1".into());
    describe("first", runtime.execute(request.clone()).await);
    request.execution_id = "exec_retry".into();
    describe("retry", runtime.execute(request).await);

    // Errors are structured for machines.
    describe(
        "invalid",
        runtime
            .execute(execution_request("create_note", json!({ "text": " " })))
            .await,
    );
    describe(
        "schema",
        runtime
            .execute(execution_request("create_note", json!({ "txt": "typo" })))
            .await,
    );

    // The same execution as a lifecycle event stream.
    let mut events = runtime.execute_stream(execution_request(
        "create_note",
        json!({ "text": "streamed" }),
    ));
    while let Some(event) = events.next().await {
        println!("event {}", event.name());
    }
    Ok(())
}
