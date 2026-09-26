//! Webhooks as tool executions. A delivery is verified on the raw body, given an
//! identity read from the payload as its `request_id`, and then executed (or enqueued)
//! like any other request. Redeliveries therefore replay instead of running twice.
//!
//! Verification is generic: an HMAC-SHA256 signature in a header, a shared secret in a
//! header, or any check the app supplies. Provider conventions live in the app.
use crate::{BODY_LIMIT, ResultDto, reusable, status_of};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use carmy_core::{AgentError, AgentResult, ErrorCategory, ExecutionRequest};
use carmy_runtime::{Runtime, execution_request};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use std::{future::Future, pin::Pin, sync::Arc};
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;
type CustomCheck = Arc<dyn Fn(&HeaderMap, &[u8]) -> bool + Send + Sync>;

/// Something that can run a request later; the facade wires the app's `Jobs` here.
pub trait Enqueue: Send + Sync {
    fn enqueue<'a>(
        &'a self,
        request: ExecutionRequest,
    ) -> Pin<Box<dyn Future<Output = AgentResult<String>> + Send + 'a>>;
}

#[derive(Clone)]
enum Verifier {
    /// A header carrying hex HMAC-SHA256 of the body, optionally prefixed `sha256=`.
    Hmac { secret: Arc<str>, header: Arc<str> },
    /// A header whose value equals a shared secret.
    SharedSecret { header: Arc<str>, secret: Arc<str> },
    /// Any check the app supplies.
    Custom(CustomCheck),
}

/// How one endpoint verifies deliveries and which tool receives them.
#[derive(Clone)]
pub struct Webhook {
    verifier: Verifier,
    tool: Option<String>,
    event_id: Option<String>,
    enqueue: bool,
}

impl Webhook {
    /// The body signed with HMAC-SHA256, hex-encoded in `header`, with or without a
    /// `sha256=` prefix.
    pub fn hmac_sha256(secret: impl Into<String>, header: impl Into<String>) -> Self {
        Self::new(Verifier::Hmac {
            secret: secret.into().into(),
            header: header.into().to_ascii_lowercase().into(),
        })
    }
    /// `header` carries a secret shared with the sender.
    pub fn shared_secret(header: impl Into<String>, secret: impl Into<String>) -> Self {
        Self::new(Verifier::SharedSecret {
            header: header.into().to_ascii_lowercase().into(),
            secret: secret.into().into(),
        })
    }
    /// Any other scheme: `check` receives the headers and the raw body and returns
    /// whether the delivery is authentic. Compare secrets in constant time.
    pub fn custom(check: impl Fn(&HeaderMap, &[u8]) -> bool + Send + Sync + 'static) -> Self {
        Self::new(Verifier::Custom(Arc::new(check)))
    }
    fn new(verifier: Verifier) -> Self {
        Self {
            verifier,
            tool: None,
            event_id: None,
            enqueue: false,
        }
    }
    /// The tool that receives the verified payload as its input. Required.
    pub fn tool(mut self, name: impl Into<String>) -> Self {
        self.tool = Some(name.into());
        self
    }
    /// A JSON pointer (`/id`, `/event/id`) to the delivery's identity, used as the
    /// `request_id` so redeliveries replay. Strings and numbers are accepted. Without
    /// it, or when the pointer finds nothing, deliveries are not deduplicated.
    pub fn event_id(mut self, pointer: impl Into<String>) -> Self {
        self.event_id = Some(pointer.into());
        self
    }
    /// Answer `202 Accepted` at once and run the tool as a job. The recommended mode:
    /// senders time out quickly and retry, and a job survives a restart.
    pub fn enqueue(mut self) -> Self {
        self.enqueue = true;
        self
    }
    /// Verify one delivery. Public so hosts with their own routes can reuse it.
    pub fn verify(&self, headers: &HeaderMap, body: &[u8]) -> Result<(), AgentError> {
        let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
        let ok = match &self.verifier {
            Verifier::Hmac { secret, header } => {
                let Some(given) = text(header) else {
                    return Err(unauthorized(format!("Missing {header} header")));
                };
                let given = given.strip_prefix("sha256=").unwrap_or(given);
                let mut mac =
                    HmacSha256::new_from_slice(secret.as_bytes()).expect("any key length");
                mac.update(body);
                hex_eq(given, &mac.finalize().into_bytes())
            }
            Verifier::SharedSecret { header, secret } => {
                let Some(given) = text(header) else {
                    return Err(unauthorized(format!("Missing {header} header")));
                };
                bool::from(given.as_bytes().ct_eq(secret.as_bytes()))
            }
            Verifier::Custom(check) => check(headers, body),
        };
        if ok {
            Ok(())
        } else {
            Err(unauthorized("Webhook signature does not match"))
        }
    }
    fn identity(&self, payload: &Value) -> Option<String> {
        match payload.pointer(self.event_id.as_deref()?)? {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }
}

fn hex_eq(given: &str, expected: &[u8]) -> bool {
    hex::decode(given.trim()).is_ok_and(|bytes| bool::from(bytes.ct_eq(expected)))
}
fn unauthorized(message: impl Into<String>) -> AgentError {
    AgentError::new("WEBHOOK_UNAUTHORIZED", message, ErrorCategory::Permission)
}

#[derive(Clone)]
struct WebhookState {
    runtime: Arc<Runtime>,
    enqueuer: Option<Arc<dyn Enqueue>>,
    hook: Arc<Webhook>,
    tool: String,
}

/// A router with one `POST` route per webhook. Merge it with [`crate::router`] (or any
/// router) and apply [`crate::harden`] to the result. `enqueuer` is required by hooks
/// that [`Webhook::enqueue`]; without one they fail at build time.
pub fn webhook_router(
    runtime: Arc<Runtime>,
    hooks: impl IntoIterator<Item = (String, Webhook)>,
    enqueuer: Option<Arc<dyn Enqueue>>,
) -> Result<Router, AgentError> {
    let mut router = Router::new();
    for (path, hook) in hooks {
        let Some(tool) = hook.tool.clone() else {
            return Err(invalid(format!("webhook `{path}` has no tool")));
        };
        if runtime.metadata(&tool).is_none() {
            return Err(invalid(format!(
                "webhook `{path}` targets unknown tool `{tool}`"
            )));
        }
        if hook.enqueue && enqueuer.is_none() {
            return Err(invalid(format!(
                "webhook `{path}` enqueues but no queue is configured"
            )));
        }
        let state = WebhookState {
            runtime: runtime.clone(),
            enqueuer: enqueuer.clone(),
            hook: Arc::new(hook),
            tool,
        };
        router = router.route(&path, post(receive).with_state(state));
    }
    Ok(router.layer(DefaultBodyLimit::max(BODY_LIMIT)))
}
fn invalid(message: String) -> AgentError {
    AgentError::new("INVALID_WEBHOOK", message, ErrorCategory::Validation)
}

async fn receive(State(state): State<WebhookState>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(error) = state.hook.verify(&headers, &body) {
        return (StatusCode::UNAUTHORIZED, Json(json!({ "error": error }))).into_response();
    }
    let payload: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            let mut error = AgentError::new(
                "INVALID_REQUEST",
                "Webhook body is not JSON",
                ErrorCategory::Validation,
            );
            error.details = Some(Box::new(json!({ "reason": e.to_string() })));
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response();
        }
    };
    let mut request = execution_request(state.tool.clone(), payload);
    request.request_id = state.hook.identity(&request.arguments);
    let no_store = [(header::CACHE_CONTROL, "no-store")];
    if state.hook.enqueue {
        let enqueuer = state.enqueuer.as_ref().expect("checked at build");
        let request_id = request.request_id.clone();
        return match enqueuer.enqueue(request).await {
            Ok(job_id) => (
                StatusCode::ACCEPTED,
                no_store,
                Json(json!({ "job_id": job_id, "request_id": request_id })),
            )
                .into_response(),
            Err(error) => (
                status_of(Some(&error)),
                no_store,
                Json(json!({ "error": error })),
            )
                .into_response(),
        };
    }
    let reusable = reusable(state.runtime.metadata(&request.tool));
    let dto = ResultDto::new(state.runtime.execute(request).await, reusable);
    (status_of(dto.error.as_ref()), no_store, Json(dto)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }

    #[test]
    fn hmac_accepts_plain_and_prefixed_hex_and_rejects_the_rest() {
        let hook = Webhook::hmac_sha256("s3cret", "X-Signature");
        let body = b"{}";
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-signature",
            format!("sha256={}", sign("s3cret", body)).parse().unwrap(),
        );
        assert!(hook.verify(&headers, body).is_ok());
        headers.insert("x-signature", sign("s3cret", body).parse().unwrap());
        assert!(hook.verify(&headers, body).is_ok());
        assert!(hook.verify(&headers, b"{\"x\":1}").is_err(), "tampered");
        headers.insert("x-signature", sign("other", body).parse().unwrap());
        assert!(hook.verify(&headers, body).is_err(), "wrong secret");
        headers.insert("x-signature", "not-hex".parse().unwrap());
        assert!(hook.verify(&headers, body).is_err());
        assert!(hook.verify(&HeaderMap::new(), body).is_err(), "missing");
    }

    #[test]
    fn shared_secret_and_custom_checks() {
        let hook = Webhook::shared_secret("X-Token", "t0ken");
        let mut headers = HeaderMap::new();
        headers.insert("x-token", "t0ken".parse().unwrap());
        assert!(hook.verify(&headers, b"").is_ok());
        headers.insert("x-token", "guess".parse().unwrap());
        assert!(hook.verify(&headers, b"").is_err());

        let hook =
            Webhook::custom(|headers, body| headers.contains_key("x-ok") && !body.is_empty());
        let mut headers = HeaderMap::new();
        assert!(hook.verify(&headers, b"{}").is_err());
        headers.insert("x-ok", "1".parse().unwrap());
        assert!(hook.verify(&headers, b"{}").is_ok());
    }

    #[test]
    fn identity_comes_from_a_json_pointer() {
        let hook = Webhook::custom(|_, _| true).event_id("/event/id");
        assert_eq!(
            hook.identity(&json!({"event": {"id": "e1"}})).as_deref(),
            Some("e1")
        );
        assert_eq!(
            hook.identity(&json!({"event": {"id": 42}})).as_deref(),
            Some("42")
        );
        assert_eq!(hook.identity(&json!({"event": {"id": ""}})), None);
        assert_eq!(hook.identity(&json!({})), None);
        assert_eq!(
            Webhook::custom(|_, _| true).identity(&json!({"id": "x"})),
            None
        );
    }
}
