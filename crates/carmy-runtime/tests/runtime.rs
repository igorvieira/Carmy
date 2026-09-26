use carmy_core::*;
use carmy_runtime::*;
use serde_json::json;
struct Echo(Effect);
impl Tool for Echo {
    type Input = String;
    type Output = String;
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: "echo".into(),
            description: "echo".into(),
            input_schema: schemars::schema_for!(String).to_value(),
            output_schema: schemars::schema_for!(String).to_value(),
            effect: self.0,
            idempotent: true,
            parallel_safe: true,
            confirmation: Confirmation::None,
        }
    }
    async fn execute(&self, _: AgentContext, input: String) -> AgentResult<String> {
        Ok(input)
    }
}
fn request(arguments: serde_json::Value) -> ExecutionRequest {
    ExecutionRequest {
        execution_id: "exec-1".into(),
        request_id: None,
        tool: "echo".into(),
        arguments,
        context: AgentContext::default(),
        metadata: Default::default(),
    }
}
#[tokio::test]
async fn execution_validation_resolution_and_policy() {
    let runtime = Runtime::new().tool(Echo(Effect::Read)).unwrap();
    assert_eq!(
        runtime.execute(request(json!("hi"))).await.outcome.unwrap(),
        "hi"
    );
    assert_eq!(
        runtime
            .execute(request(json!(4)))
            .await
            .outcome
            .unwrap_err()
            .code,
        "INVALID_ARGUMENTS"
    );
    let mut missing = request(json!("hi"));
    missing.tool = "missing".into();
    assert_eq!(
        runtime.execute(missing).await.outcome.unwrap_err().code,
        "TOOL_NOT_FOUND"
    );
    let runtime = Runtime::new().tool(Echo(Effect::Destructive)).unwrap();
    assert_eq!(
        runtime
            .execute(request(json!("hi")))
            .await
            .outcome
            .unwrap_err()
            .code,
        "CONFIRMATION_REQUIRED"
    );
    let mut confirmed = request(json!("hi"));
    confirmed.context.permissions.insert("confirm:echo".into());
    assert!(runtime.execute(confirmed).await.outcome.is_ok());
}
#[test]
fn duplicate_names_are_rejected() {
    assert!(
        Runtime::new()
            .tool(Echo(Effect::Read))
            .unwrap()
            .tool(Echo(Effect::Read))
            .is_err()
    );
}

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
struct Write {
    calls: Arc<AtomicUsize>,
    delay: Duration,
}
impl Tool for Write {
    type Input = String;
    type Output = String;
    fn metadata(&self) -> ToolMetadata {
        let mut m = Echo(Effect::Write).metadata();
        m.idempotent = false;
        m
    }
    async fn execute(&self, _: AgentContext, input: String) -> AgentResult<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        Ok(input)
    }
}
#[tokio::test]
async fn retries_conflicts_and_scopes() {
    let calls = Arc::new(AtomicUsize::new(0));
    let rt = Runtime::new()
        .tool(Write {
            calls: calls.clone(),
            delay: Duration::ZERO,
        })
        .unwrap();
    let mut req = request(json!("write"));
    req.request_id = Some("abc".into());
    let a = rt.execute(req.clone()).await;
    let b = rt.execute(req.clone()).await;
    assert_eq!(a, b);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    req.arguments = json!("changed");
    assert_eq!(
        rt.execute(req.clone()).await.outcome.unwrap_err().code,
        "IDEMPOTENCY_CONFLICT"
    );
    req.context.principal = Some("another".into());
    assert!(rt.execute(req).await.outcome.is_ok());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn concurrent_duplicates_are_reserved_atomically() {
    let calls = Arc::new(AtomicUsize::new(0));
    let rt = Runtime::new()
        .tool(Write {
            calls: calls.clone(),
            delay: Duration::from_millis(20),
        })
        .unwrap();
    let mut req = request(json!("write"));
    req.request_id = Some("abc".into());
    let (a, b) = tokio::join!(rt.execute(req.clone()), rt.execute(req));
    assert!(a.outcome.is_ok());
    assert_eq!(b.outcome.unwrap_err().code, "EXECUTION_UNCERTAIN");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn timeout_and_cancellation_never_repeat_uncertain_writes() {
    let calls = Arc::new(AtomicUsize::new(0));
    let rt = Runtime::new()
        .timeout(Duration::from_millis(5))
        .tool(Write {
            calls: calls.clone(),
            delay: Duration::from_secs(10),
        })
        .unwrap();
    let mut req = request(json!("write"));
    req.request_id = Some("abc".into());
    let timed = rt.execute(req.clone()).await;
    assert_eq!(timed.status, ExecutionStatus::TimedOut);
    // New transport request has its own cancellation token.
    req.context = AgentContext::default();
    assert_eq!(rt.execute(req).await, timed);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let req = request(json!("write"));
    req.context.cancellation.cancel();
    assert_eq!(rt.execute(req).await.status, ExecutionStatus::Cancelled);
}
#[tokio::test]
async fn dropping_execution_cancels_and_retains_reservation() {
    let calls = Arc::new(AtomicUsize::new(0));
    let rt = Runtime::new()
        .tool(Write {
            calls: calls.clone(),
            delay: Duration::from_secs(10),
        })
        .unwrap();
    let mut req = request(json!("write"));
    req.request_id = Some("abc".into());
    let token = req.context.cancellation.clone();
    {
        let execution = rt.execute(req.clone());
        tokio::pin!(execution);
        tokio::select! { _ = &mut execution => panic!("unexpected completion"), _ = tokio::time::sleep(Duration::from_millis(5)) => {} }
    }
    assert!(token.is_cancelled());
    req.context = AgentContext::default();
    assert_eq!(
        rt.execute(req).await.outcome.unwrap_err().code,
        "EXECUTION_UNCERTAIN"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
fn names(events: &[ExecutionEvent]) -> Vec<&'static str> {
    events.iter().map(ExecutionEvent::name).collect()
}
#[tokio::test]
async fn event_stream_reports_lifecycle_and_replays() {
    use futures_util::StreamExt;
    let calls = Arc::new(AtomicUsize::new(0));
    let rt = Arc::new(
        Runtime::new()
            .tool(Write {
                calls: calls.clone(),
                delay: Duration::ZERO,
            })
            .unwrap(),
    );
    let mut req = request(json!("write"));
    req.request_id = Some("abc".into());
    let first: Vec<_> = rt.execute_stream(req.clone()).collect().await;
    assert_eq!(
        names(&first),
        [
            "execution.started",
            "tool.started",
            "tool.completed",
            "execution.completed"
        ]
    );
    let ExecutionEvent::ExecutionCompleted { result, replayed } = first.last().unwrap() else {
        panic!("last event must complete the execution")
    };
    assert!(!replayed);
    assert_eq!(result.status, ExecutionStatus::Completed);
    req.context = AgentContext::default();
    let replay: Vec<_> = rt.execute_stream(req).collect().await;
    assert_eq!(names(&replay), ["execution.started", "execution.completed"]);
    assert!(matches!(
        replay.last(),
        Some(ExecutionEvent::ExecutionCompleted { replayed: true, .. })
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn dropping_event_stream_cancels_execution() {
    use futures_util::StreamExt;
    let rt = Arc::new(
        Runtime::new()
            .tool(Write {
                calls: Arc::default(),
                delay: Duration::from_secs(10),
            })
            .unwrap(),
    );
    let req = request(json!("write"));
    let token = req.context.cancellation.clone();
    let mut stream = rt.execute_stream(req);
    assert_eq!(stream.next().await.unwrap().name(), "execution.started");
    assert_eq!(stream.next().await.unwrap().name(), "tool.started");
    drop(stream);
    assert!(token.is_cancelled());
}
#[test]
fn execution_ids_are_unique_across_threads() {
    let ids: Vec<String> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    (0..10_000)
                        .map(|_| execution_request("echo", json!("x")).execution_id)
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect()
    });
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len());
    assert!(
        ids.iter()
            .all(|id| id.starts_with("exec_") && id.len() == 41)
    );
}
struct Panics {
    after_await: bool,
}
impl Tool for Panics {
    type Input = String;
    type Output = String;
    fn metadata(&self) -> ToolMetadata {
        let mut m = Echo(Effect::Read).metadata();
        m.name = "panics".into();
        m
    }
    async fn execute(&self, _: AgentContext, _: String) -> AgentResult<String> {
        if self.after_await {
            tokio::task::yield_now().await;
        }
        panic!("tool bug");
    }
}
#[tokio::test]
async fn panics_are_isolated_on_every_path() {
    for after_await in [false, true] {
        let rt = Runtime::new().tool(Panics { after_await }).unwrap();
        let mut req = request(json!("x"));
        req.tool = "panics".into();
        let result = rt.execute(req).await;
        assert_eq!(result.status, ExecutionStatus::Failed);
        assert_eq!(result.outcome.unwrap_err().code, "TOOL_PANIC");
    }
}
#[tokio::test]
async fn deadline_applies_once_a_tool_suspends() {
    let rt = Runtime::new()
        .timeout(Duration::from_millis(20))
        .tool(Write {
            calls: Arc::default(),
            delay: Duration::from_secs(5),
        })
        .unwrap();
    let started = std::time::Instant::now();
    let result = rt.execute(request(json!("x"))).await;
    assert_eq!(result.outcome.unwrap_err().code, "TIMEOUT");
    assert!(started.elapsed() < Duration::from_secs(1));
}
#[tokio::test]
async fn rate_limits_per_principal_with_retry_after() {
    let rt = Runtime::new()
        .policy(RateLimit::new(2, Duration::from_millis(300)))
        .tool(Echo(Effect::Read))
        .unwrap();
    let as_user = |name: &str| {
        let mut req = request(json!("x"));
        req.context.principal = Some(name.into());
        req
    };
    assert!(rt.execute(as_user("ada")).await.outcome.is_ok());
    assert!(rt.execute(as_user("ada")).await.outcome.is_ok());
    let denied = rt.execute(as_user("ada")).await.outcome.unwrap_err();
    assert_eq!(denied.code, "RATE_LIMITED");
    assert_eq!(denied.category, ErrorCategory::Capacity);
    assert!(denied.retryable && denied.retry_after.is_some());
    // Another principal has its own window; anonymous callers share one.
    assert!(rt.execute(as_user("bob")).await.outcome.is_ok());
    assert!(rt.execute(request(json!("x"))).await.outcome.is_ok());
    assert!(rt.execute(request(json!("x"))).await.outcome.is_ok());
    assert_eq!(
        rt.execute(request(json!("x")))
            .await
            .outcome
            .unwrap_err()
            .code,
        "RATE_LIMITED"
    );
    // The window passes and the principal is allowed again.
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(rt.execute(as_user("ada")).await.outcome.is_ok());
}
