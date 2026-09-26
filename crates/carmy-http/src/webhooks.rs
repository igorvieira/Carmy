//! Webhooks as tool executions. A provider's delivery is verified on the raw body, given
//! the provider's event id as `request_id`, and then executed (or enqueued) like any
//! other request. Redeliveries therefore replay instead of running twice.
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
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;
type EventId = Arc<dyn Fn(&Value) -> Option<String> + Send + Sync>;

/// Something that can run a request later; the facade wires the app's `Jobs` here.
pub trait Enqueue: Send + Sync {
    fn enqueue<'a>(
        &'a self,
        request: ExecutionRequest,
    ) -> Pin<Box<dyn Future<Output = AgentResult<String>> + Send + 'a>>;
}

#[derive(Clone)]
enum Verifier {
    /// `Stripe-Signature: t=<unix>,v1=<hex>` over `"{t}.{body}"`, within `tolerance`.
    Stripe {
        secret: Arc<str>,
        tolerance: Duration,
    },
    /// `X-Telegram-Bot-Api-Secret-Token` equals the configured token.
    Telegram { token: Arc<str> },
    /// A header carrying hex HMAC-SHA256 of the body, optionally prefixed `sha256=`.
    Hmac { secret: Arc<str>, header: Arc<str> },
}

/// How one endpoint verifies deliveries and which tool receives them.
#[derive(Clone)]
pub struct Webhook {
    verifier: Verifier,
    tool: Option<String>,
    event_id: EventId,
    enqueue: bool,
}

impl Webhook {
    /// Stripe: signed `Stripe-Signature`, `request_id` from the event `id`.
    pub fn stripe(secret: impl Into<String>) -> Self {
        Self::new(
            Verifier::Stripe {
                secret: secret.into().into(),
                tolerance: Duration::from_secs(300),
            },
            Arc::new(|event| event.get("id")?.as_str().map(str::to_owned)),
        )
    }
    /// Telegram: the bot's secret token header, `request_id` from `update_id`.
    pub fn telegram(secret_token: impl Into<String>) -> Self {
        Self::new(
            Verifier::Telegram {
                token: secret_token.into().into(),
            },
            Arc::new(|update| update.get("update_id")?.as_u64().map(|id| id.to_string())),
        )
    }
    /// Any provider signing the body with HMAC-SHA256 in `header` (GitHub, Shopify...).
    /// Set [`Webhook::event_id`] so redeliveries replay.
    pub fn hmac_sha256(secret: impl Into<String>, header: impl Into<String>) -> Self {
        Self::new(
            Verifier::Hmac {
                secret: secret.into().into(),
                header: header.into().to_ascii_lowercase().into(),
            },
            Arc::new(|_| None),
        )
    }
    fn new(verifier: Verifier, event_id: EventId) -> Self {
        Self {
            verifier,
            tool: None,
            event_id,
            enqueue: false,
        }
    }
    /// The tool that receives the verified payload as its input. Required.
    pub fn tool(mut self, name: impl Into<String>) -> Self {
        self.tool = Some(name.into());
        self
    }
    /// Where the delivery's identity lives, for providers without a convention.
    pub fn event_id(
        mut self,
        extract: impl Fn(&Value) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.event_id = Arc::new(extract);
        self
    }
    /// Accepted tolerance between the signature timestamp and now (Stripe only).
    pub fn tolerance(mut self, tolerance: Duration) -> Self {
        if let Verifier::Stripe { tolerance: t, .. } = &mut self.verifier {
            *t = tolerance;
        }
        self
    }
    /// Answer `202 Accepted` at once and run the tool as a job. The recommended mode:
    /// providers time out quickly and retry, and a job survives a restart.
    pub fn enqueue(mut self) -> Self {
        self.enqueue = true;
        self
    }
    /// Verify one delivery. Public so hosts with their own routes can reuse it.
    pub fn verify(&self, headers: &HeaderMap, body: &[u8]) -> Result<(), AgentError> {
        self.verify_at(headers, body, unix_now())
    }
    fn verify_at(&self, headers: &HeaderMap, body: &[u8], now: u64) -> Result<(), AgentError> {
        let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
        let ok = match &self.verifier {
            Verifier::Stripe { secret, tolerance } => {
                let Some(signature) = text("stripe-signature") else {
                    return Err(unauthorized("Missing Stripe-Signature header"));
                };
                let mut timestamp = None;
                let mut signatures = Vec::new();
                for part in signature.split(',') {
                    match part.trim().split_once('=') {
                        Some(("t", t)) => timestamp = t.parse::<u64>().ok(),
                        Some(("v1", v)) => signatures.push(v),
                        _ => {}
                    }
                }
                let Some(timestamp) = timestamp else {
                    return Err(unauthorized("Malformed Stripe-Signature header"));
                };
                if now.abs_diff(timestamp) > tolerance.as_secs() {
                    return Err(unauthorized("Stripe-Signature timestamp outside tolerance"));
                }
                let mut mac =
                    HmacSha256::new_from_slice(secret.as_bytes()).expect("any key length");
                mac.update(timestamp.to_string().as_bytes());
                mac.update(b".");
                mac.update(body);
                let expected = mac.finalize().into_bytes();
                signatures.iter().any(|s| hex_eq(s, &expected))
            }
            Verifier::Telegram { token } => text("x-telegram-bot-api-secret-token")
                .is_some_and(|given| bool::from(given.as_bytes().ct_eq(token.as_bytes()))),
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
        };
        if ok {
            Ok(())
        } else {
            Err(unauthorized("Webhook signature does not match"))
        }
    }
}

fn hex_eq(given: &str, expected: &[u8]) -> bool {
    hex::decode(given.trim()).is_ok_and(|bytes| bool::from(bytes.ct_eq(expected)))
}
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
    request.request_id = (state.hook.event_id)(&request.arguments);
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

    fn stripe_header(secret: &str, t: u64, body: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("{t}.{body}").as_bytes());
        format!("t={t},v1={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn stripe_signatures_are_checked_with_tolerance() {
        let hook = Webhook::stripe("whsec_test");
        let body = r#"{"id":"evt_1"}"#;
        let mut headers = HeaderMap::new();
        headers.insert(
            "stripe-signature",
            stripe_header("whsec_test", 1_000, body).parse().unwrap(),
        );
        assert!(hook.verify_at(&headers, body.as_bytes(), 1_100).is_ok());
        assert!(
            hook.verify_at(&headers, body.as_bytes(), 1_400).is_err(),
            "expired"
        );
        assert!(
            hook.verify_at(&headers, b"{\"id\":\"evt_2\"}", 1_100)
                .is_err(),
            "tampered"
        );
        headers.insert(
            "stripe-signature",
            stripe_header("other", 1_000, body).parse().unwrap(),
        );
        assert!(
            hook.verify_at(&headers, body.as_bytes(), 1_100).is_err(),
            "wrong secret"
        );
        assert!(
            hook.verify_at(&HeaderMap::new(), body.as_bytes(), 1_100)
                .is_err(),
            "missing"
        );
    }

    #[test]
    fn generic_hmac_accepts_prefixed_hex() {
        let hook = Webhook::hmac_sha256("s3cret", "X-Hub-Signature-256");
        let body = b"{}";
        let mut mac = HmacSha256::new_from_slice(b"s3cret").unwrap();
        mac.update(body);
        let hex = hex::encode(mac.finalize().into_bytes());
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-hub-signature-256",
            format!("sha256={hex}").parse().unwrap(),
        );
        assert!(hook.verify(&headers, body).is_ok());
        headers.insert("x-hub-signature-256", hex.parse().unwrap());
        assert!(hook.verify(&headers, body).is_ok());
        headers.insert("x-hub-signature-256", "sha256=00".parse().unwrap());
        assert!(hook.verify(&headers, body).is_err());
    }
}
