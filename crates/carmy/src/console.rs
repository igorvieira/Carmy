//! The `carmy-console/1` protocol: JSON Lines over any byte stream.
//!
//! Every request goes through the runtime, with its policies, validation, idempotency,
//! deadlines and tracing. Agents speak the protocol directly
//! (`cargo run -- console`); `carmy console` draws a terminal UI on top of it.
//!
//! One JSON object per line in, one per line out:
//!
//! ```text
//! {"event":"ready","protocol":"carmy-console/1","server":"shop","tools":3}
//! {"id":1,"op":"tools"}                                    -> {"id":1,"ok":true,"result":{"tools":[…]}}
//! {"id":2,"op":"describe","tool":"search"}                 -> {"id":2,"ok":true,"result":{…metadata}}
//! {"id":3,"op":"call","tool":"search","arguments":{…},"request_id":"r1"}
//!                                                          -> {"id":3,"ok":true,"result":{"status":…,"replayed":false,…}}
//! {"id":4,"op":"confirm","tool":"delete"}                  -> grants confirm:delete for this session
//! {"id":5,"op":"revoke","tool":"delete"}
//! {"id":6,"op":"audit","limit":20}                        -> the latest execution records
//! {"id":7,"op":"dead","limit":20}                         -> jobs in the dead-letter queue
//! ```
//!
//! A line that does not start with `{` is read as a text command (`tools`,
//! `describe <tool>`, `confirm <tool>`, `revoke <tool>`, `audit [n]`, `dead [n]`, `help`, `exit`, or
//! `<tool> [json] [--request-id <id>]`); the answer is still JSON. Unstable during 0.x.
use crate::{
    AgentContext, AgentError, CancellationToken, ErrorCategory, ExecutionEvent, ExecutionStatus,
    runtime::{InMemoryAudit, Runtime, execution_request},
};
use futures_util::StreamExt;
use serde_json::{Map, Value, json};
use std::{sync::Arc, time::Instant};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

/// Protocol name sent in the `ready` event.
pub const PROTOCOL: &str = "carmy-console/1";

/// What the console can show beyond the tools: the audit trail and the job queue.
#[derive(Default)]
pub struct Extras {
    pub audit: Option<Arc<InMemoryAudit>>,
    pub jobs: Option<crate::jobs::Jobs>,
}

/// Serve the protocol until `input` ends or an `exit` request arrives.
pub async fn serve<R, W>(
    runtime: Arc<Runtime>,
    server: &str,
    input: R,
    output: W,
) -> std::io::Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    serve_with(runtime, server, Extras::default(), input, output).await
}

/// [`serve`], with the audit trail and job queue available to `audit` and `dead`.
pub async fn serve_with<R, W>(
    runtime: Arc<Runtime>,
    server: &str,
    extras: Extras,
    input: R,
    mut output: W,
) -> std::io::Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut session = Session::new(runtime, extras);
    let ready = json!({
        "event": "ready",
        "protocol": PROTOCOL,
        "server": server,
        "tools": session.runtime.tools().len(),
    });
    write_line(&mut output, &ready).await?;
    let mut lines = input.lines();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let request = match parse(line) {
            Ok(request) => request,
            Err(error) => {
                write_line(&mut output, &failure(None, error)).await?;
                continue;
            }
        };
        if request.get("op").and_then(Value::as_str) == Some("exit") {
            break;
        }
        let response = session.handle(request).await;
        write_line(&mut output, &response).await?;
    }
    output.flush().await
}

struct Session {
    runtime: Arc<Runtime>,
    context: AgentContext,
    extras: Extras,
}

impl Session {
    fn new(runtime: Arc<Runtime>, extras: Extras) -> Self {
        let context = AgentContext {
            principal: Some("console".into()),
            session: Some("console".into()),
            ..AgentContext::default()
        };
        Self {
            runtime,
            context,
            extras,
        }
    }

    async fn handle(&mut self, request: Value) -> Value {
        let id = request.get("id").cloned();
        let tool = request
            .get("tool")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let outcome = match request.get("op").and_then(Value::as_str) {
            Some("help") => Ok(help()),
            Some("tools") => Ok(json!({ "tools": self.runtime.tools() })),
            Some("describe") => self.known(tool.as_deref()).map(|m| json!(m)),
            Some("confirm") | Some("revoke") => {
                let grant = request["op"] == "confirm";
                self.known(tool.as_deref()).map(|m| {
                    let permission = format!("confirm:{}", m.name);
                    if grant {
                        self.context.permissions.insert(permission);
                    } else {
                        self.context.permissions.remove(&permission);
                    }
                    json!({ "tool": m.name, "confirmed": grant })
                })
            }
            Some("call") => match tool {
                Some(tool) => Ok(self.call(tool, &request).await),
                None => Err(invalid("`call` needs a `tool`")),
            },
            Some("audit") => match &self.extras.audit {
                Some(audit) => Ok(json!({ "executions": audit.recent(limit(&request)) })),
                None => Err(unavailable("no audit trail is attached to this console")),
            },
            Some("dead") => match &self.extras.jobs {
                Some(jobs) => jobs
                    .dead_letters(limit(&request))
                    .await
                    .map(|jobs| json!({ "jobs": jobs })),
                None => Err(unavailable("no job queue is attached to this console")),
            },
            Some(other) => Err(AgentError::new(
                "UNKNOWN_OP",
                format!("unknown op `{other}`"),
                ErrorCategory::Validation,
            )
            .recoverable()
            .suggest("help")),
            None => Err(invalid("a request needs an `op`")),
        };
        match outcome {
            Ok(result) => {
                let mut response = Map::new();
                if let Some(id) = id {
                    response.insert("id".into(), id);
                }
                response.insert("ok".into(), true.into());
                response.insert("result".into(), result);
                Value::Object(response)
            }
            Err(error) => failure(id, error),
        }
    }

    fn known(&self, tool: Option<&str>) -> Result<crate::ToolMetadata, AgentError> {
        let tool = tool.ok_or_else(|| invalid("this op needs a `tool`"))?;
        self.runtime.metadata(tool).cloned().ok_or_else(|| {
            AgentError::new(
                "TOOL_NOT_FOUND",
                format!("unknown tool `{tool}`"),
                ErrorCategory::NotFound,
            )
            .recoverable()
            .suggest("tools")
        })
    }

    async fn call(&self, tool: String, request: &Value) -> Value {
        let arguments = request
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let mut execution = execution_request(tool.clone(), arguments);
        execution.request_id = request
            .get("request_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        execution.context = self.context.clone();
        // Each call gets its own token: cancelling one never cancels the session.
        execution.context.cancellation = CancellationToken::new();
        let started = Instant::now();
        let mut events = self.runtime.execute_stream(execution);
        let mut last = None;
        while let Some(event) = events.next().await {
            last = Some(event);
        }
        let duration_ms = started.elapsed().as_secs_f64() * 1000.0;
        let Some(ExecutionEvent::ExecutionCompleted { result, replayed }) = last else {
            unreachable!("an execution stream always ends with ExecutionCompleted");
        };
        let reusable = self.runtime.metadata(&tool).is_some_and(|m| {
            m.idempotent && matches!(m.effect, crate::Effect::None | crate::Effect::Read)
        });
        let mut dto = Map::new();
        dto.insert("execution_id".into(), result.execution_id.into());
        dto.insert("status".into(), result.status.as_str().into());
        let ok = result.status == ExecutionStatus::Completed;
        match result.outcome {
            Ok(data) => dto.insert("data".into(), data),
            Err(error) => dto.insert("error".into(), json!(error)),
        };
        dto.insert(
            "_agent".into(),
            json!({ "cacheable": reusable && ok, "next_actions": [] }),
        );
        dto.insert("replayed".into(), replayed.into());
        dto.insert(
            "duration_ms".into(),
            json!((duration_ms * 1000.0).round() / 1000.0),
        );
        Value::Object(dto)
    }
}

fn limit(request: &Value) -> usize {
    request
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(20, |n| n.clamp(1, 1_000) as usize)
}

fn unavailable(message: &str) -> AgentError {
    AgentError::new("UNAVAILABLE", message, ErrorCategory::NotFound).recoverable()
}

fn invalid(message: &str) -> AgentError {
    AgentError::new("INVALID_REQUEST", message, ErrorCategory::Validation)
        .recoverable()
        .suggest("help")
}

fn failure(id: Option<Value>, error: AgentError) -> Value {
    let mut response = Map::new();
    if let Some(id) = id {
        response.insert("id".into(), id);
    }
    response.insert("ok".into(), false.into());
    response.insert("error".into(), json!(error));
    Value::Object(response)
}

fn help() -> Value {
    json!({
        "protocol": PROTOCOL,
        "ops": {
            "tools": "list the tools",
            "describe": "a tool's metadata and schemas; needs `tool`",
            "call": "run a tool; needs `tool`, optional `arguments` and `request_id`",
            "confirm": "grant confirm:<tool> for this session; needs `tool`",
            "revoke": "withdraw confirm:<tool>; needs `tool`",
            "audit": "the latest execution records, newest first; optional `limit`",
            "dead": "jobs in the dead-letter queue; optional `limit`",
            "exit": "end the session",
        },
        "text": "tools | describe <tool> | confirm <tool> | revoke <tool> | audit [n] | dead [n] | help | exit | <tool> [json] [--request-id <id>]",
    })
}

/// A JSON request, or a text command turned into one.
fn parse(line: &str) -> Result<Value, AgentError> {
    if line.starts_with('{') {
        return serde_json::from_str(line).map_err(|e| {
            invalid("the line is not valid JSON").details(json!({ "reason": e.to_string() }))
        });
    }
    let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let rest = rest.trim();
    let named = |op: &str| {
        if rest.is_empty() {
            Err(invalid(&format!("`{op}` needs a tool name")))
        } else {
            Ok(json!({ "op": op, "tool": rest }))
        }
    };
    match word {
        "tools" | "help" | "exit" | "quit" => {
            Ok(json!({ "op": if word == "quit" { "exit" } else { word } }))
        }
        "describe" | "confirm" | "revoke" => named(word),
        "audit" | "dead" => {
            let mut request = json!({ "op": word });
            if !rest.is_empty() {
                let n: u64 = rest
                    .parse()
                    .map_err(|_| invalid(&format!("`{word}` takes a number, e.g. {word} 50")))?;
                request["limit"] = n.into();
            }
            Ok(request)
        }
        tool => {
            let (arguments, request_id) = match rest.rsplit_once("--request-id") {
                Some((arguments, id)) => (arguments.trim(), Some(id.trim())),
                None => (rest, None),
            };
            let arguments: Value = if arguments.is_empty() {
                json!({})
            } else {
                serde_json::from_str(arguments).map_err(|e| {
                    invalid("arguments must be JSON, e.g. search {\"query\": \"mouse\"}")
                        .details(json!({ "reason": e.to_string() }))
                })?
            };
            let mut request = json!({ "op": "call", "tool": tool, "arguments": arguments });
            if let Some(id) = request_id.filter(|id| !id.is_empty()) {
                request["request_id"] = id.into();
            }
            Ok(request)
        }
    }
}

async fn write_line<W: AsyncWrite + Unpin>(output: &mut W, value: &Value) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(value).expect("JSON values serialize");
    line.push(b'\n');
    output.write_all(&line).await?;
    output.flush().await
}
