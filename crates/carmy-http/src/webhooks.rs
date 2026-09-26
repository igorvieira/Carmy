//! Webhooks: a door into a tool. A delivery is verified by the app's check, given an
//! identity read from the payload as its `request_id`, and then executed (or enqueued)
//! like any other request, so redeliveries replay instead of running twice.
//!
//! Carmy knows no sender. [`verify`] holds two common checks; anything else is a
//! closure over the headers and the raw body.
use crate::{BODY_LIMIT, DISCOVERY_PATH, EXECUTE_PATH, ResultDto, TOOLS_PATH, reusable, status_of};
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
use serde_json::{Value, json};
use std::{future::Future, pin::Pin, sync::Arc};

type Check = Arc<dyn Fn(&Delivery) -> bool + Send + Sync>;

/// Something that can run a request later; the facade wires the app's `Jobs` here.
pub trait Enqueue: Send + Sync {
    fn enqueue<'a>(
        &'a self,
        request: ExecutionRequest,
    ) -> Pin<Box<dyn Future<Output = AgentResult<String>> + Send + 'a>>;
}

/// What a verification check sees: the request headers and the raw body, before any
/// parsing.
pub struct Delivery<'a> {
    pub headers: &'a HeaderMap,
    pub body: &'a [u8],
}

impl Delivery<'_> {
    /// A header as text, if present and valid UTF-8.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

/// Common checks for [`Webhook::verify`].
pub mod verify {
    use super::Delivery;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use subtle::ConstantTimeEq;

    /// `header` holds the hex HMAC-SHA256 of the body, with or without `sha256=`.
    pub fn hmac_sha256(
        secret: impl Into<String>,
        header: impl Into<String>,
    ) -> impl Fn(&Delivery) -> bool + Send + Sync + 'static {
        let (secret, header) = (secret.into(), header.into().to_ascii_lowercase());
        move |delivery| {
            let Some(given) = delivery.header(&header) else {
                return false;
            };
            let given = given.strip_prefix("sha256=").unwrap_or(given);
            let mut mac =
                Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("any key length");
            mac.update(delivery.body);
            let expected = mac.finalize().into_bytes();
            hex::decode(given.trim()).is_ok_and(|bytes| bool::from(bytes.ct_eq(&expected)))
        }
    }

    /// `header` equals a secret shared with the sender.
    pub fn shared_secret(
        header: impl Into<String>,
        secret: impl Into<String>,
    ) -> impl Fn(&Delivery) -> bool + Send + Sync + 'static {
        let (header, secret) = (header.into().to_ascii_lowercase(), secret.into());
        move |delivery| {
            delivery
                .header(&header)
                .is_some_and(|given| bool::from(given.as_bytes().ct_eq(secret.as_bytes())))
        }
    }
}

/// Where one webhook leads and how its deliveries are checked.
#[derive(Clone)]
pub struct Webhook {
    tool: String,
    check: Option<Check>,
    unverified: bool,
    event_id: Option<String>,
    enqueue: bool,
}

impl Webhook {
    /// A webhook whose verified payload becomes the input of `tool`.
    pub fn to(tool: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            check: None,
            unverified: false,
            event_id: None,
            enqueue: false,
        }
    }
    /// How to tell an authentic delivery: a [`verify`] helper or any closure. Compare
    /// secrets in constant time. Required unless [`Webhook::unverified`].
    pub fn verify(mut self, check: impl Fn(&Delivery) -> bool + Send + Sync + 'static) -> Self {
        self.check = Some(Arc::new(check));
        self
    }
    /// Accept every delivery. Only for senders already authenticated upstream, such as
    /// a private network or a gateway.
    pub fn unverified(mut self) -> Self {
        self.unverified = true;
        self
    }
    /// A JSON pointer (`/id`, `/event/id`) to the delivery's identity, used as the
    /// `request_id` so redeliveries replay. Strings and numbers are accepted. Without
    /// it, or when the pointer finds nothing, every delivery runs.
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
    /// Check one delivery. Public so hosts with their own routes can reuse it.
    pub fn check(&self, headers: &HeaderMap, body: &[u8]) -> Result<(), AgentError> {
        let authentic = match &self.check {
            Some(check) => check(&Delivery { headers, body }),
            None => self.unverified,
        };
        if authentic {
            Ok(())
        } else {
            Err(AgentError::new(
                "WEBHOOK_UNAUTHORIZED",
                "The delivery did not pass the webhook's verification",
                ErrorCategory::Permission,
            ))
        }
    }
    /// What agents see in discovery and the console. Never the secret.
    pub fn describe(&self, path: &str) -> Value {
        json!({
            "path": path,
            "tool": self.tool,
            "mode": if self.enqueue { "enqueue" } else { "inline" },
            "event_id": self.event_id,
            "verified": self.check.is_some(),
        })
    }
    fn identity(&self, payload: &Value) -> Option<String> {
        match payload.pointer(self.event_id.as_deref()?)? {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }
}

#[derive(Clone)]
struct WebhookState {
    runtime: Arc<Runtime>,
    enqueuer: Option<Arc<dyn Enqueue>>,
    hook: Arc<Webhook>,
}

/// One `POST` route per webhook, checked before anything is mounted: a tool must exist,
/// a check or [`Webhook::unverified`] must be set, a queue must exist for
/// [`Webhook::enqueue`], and paths must be free.
pub(crate) fn webhook_routes(
    runtime: &Arc<Runtime>,
    hooks: Vec<(String, Webhook)>,
    enqueuer: Option<Arc<dyn Enqueue>>,
) -> Result<Router, AgentError> {
    let mut router = Router::new();
    let mut seen = std::collections::HashSet::new();
    for (path, hook) in hooks {
        let fail = |why: &str| Err(invalid(format!("webhook `{path}` {why}")));
        if !path.starts_with('/') {
            return fail("must start with `/`");
        }
        if [
            DISCOVERY_PATH,
            TOOLS_PATH,
            EXECUTE_PATH,
            crate::HEALTH_PATH,
            crate::READY_PATH,
        ]
        .contains(&path.as_str())
            || !seen.insert(path.clone())
        {
            return fail("uses a path that is already taken");
        }
        if runtime.metadata(&hook.tool).is_none() {
            return fail(&format!("targets unknown tool `{}`", hook.tool));
        }
        if hook.check.is_none() && !hook.unverified {
            return fail("has no verification; add .verify(..), or .unverified() on purpose");
        }
        if hook.enqueue && enqueuer.is_none() {
            return fail("enqueues but no queue is configured");
        }
        let state = WebhookState {
            runtime: runtime.clone(),
            enqueuer: enqueuer.clone(),
            hook: Arc::new(hook),
        };
        router = router.route(&path, post(receive).with_state(state));
    }
    Ok(router.layer(DefaultBodyLimit::max(BODY_LIMIT)))
}
fn invalid(message: String) -> AgentError {
    AgentError::new("INVALID_WEBHOOK", message, ErrorCategory::Validation)
}

async fn receive(State(state): State<WebhookState>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(error) = state.hook.check(&headers, &body) {
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
    let mut request = execution_request(state.hook.tool.clone(), payload);
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
    use hmac::{Hmac, Mac};

    fn sign(secret: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }

    #[test]
    fn hmac_accepts_plain_and_prefixed_hex_and_rejects_the_rest() {
        let hook = Webhook::to("t").verify(verify::hmac_sha256("s3cret", "X-Signature"));
        let body = b"{}";
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-signature",
            format!("sha256={}", sign("s3cret", body)).parse().unwrap(),
        );
        assert!(hook.check(&headers, body).is_ok());
        headers.insert("x-signature", sign("s3cret", body).parse().unwrap());
        assert!(hook.check(&headers, body).is_ok());
        assert!(hook.check(&headers, b"{\"x\":1}").is_err(), "tampered");
        headers.insert("x-signature", sign("other", body).parse().unwrap());
        assert!(hook.check(&headers, body).is_err(), "wrong secret");
        headers.insert("x-signature", "not-hex".parse().unwrap());
        assert!(hook.check(&headers, body).is_err());
        assert!(hook.check(&HeaderMap::new(), body).is_err(), "missing");
    }

    #[test]
    fn shared_secrets_closures_and_unverified() {
        let hook = Webhook::to("t").verify(verify::shared_secret("X-Token", "t0ken"));
        let mut headers = HeaderMap::new();
        headers.insert("x-token", "t0ken".parse().unwrap());
        assert!(hook.check(&headers, b"").is_ok());
        headers.insert("x-token", "guess".parse().unwrap());
        assert!(hook.check(&headers, b"").is_err());

        let hook = Webhook::to("t").verify(|d| d.header("x-ok").is_some() && !d.body.is_empty());
        let mut headers = HeaderMap::new();
        assert!(hook.check(&headers, b"{}").is_err());
        headers.insert("x-ok", "1".parse().unwrap());
        assert!(hook.check(&headers, b"{}").is_ok());

        assert!(
            Webhook::to("t").check(&HeaderMap::new(), b"").is_err(),
            "no check, no entry"
        );
        assert!(
            Webhook::to("t")
                .unverified()
                .check(&HeaderMap::new(), b"")
                .is_ok()
        );
    }

    #[test]
    fn identity_comes_from_a_json_pointer() {
        let hook = Webhook::to("t").event_id("/event/id");
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
        assert_eq!(Webhook::to("t").identity(&json!({"id": "x"})), None);
    }

    #[test]
    fn descriptions_never_carry_secrets() {
        let hook = Webhook::to("billing_event")
            .verify(verify::hmac_sha256("s3cret", "X-Signature"))
            .event_id("/id")
            .enqueue();
        let described = hook.describe("/webhooks/billing");
        assert_eq!(
            described,
            json!({
                "path": "/webhooks/billing",
                "tool": "billing_event",
                "mode": "enqueue",
                "event_id": "/id",
                "verified": true,
            })
        );
        assert!(!described.to_string().contains("s3cret"));
    }
}
