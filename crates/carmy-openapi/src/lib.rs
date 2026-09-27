//! An OpenAPI 3 description as Carmy tools: one tool per operation, with an effect,
//! a confirmation rule, a validated input schema and replay-safe retries, before any
//! agent touches the API.
//!
//! ```no_run
//! # fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let tools = carmy_openapi::OpenApi::from_file("petstore.json")?
//!     .base_url("https://api.example.com/v1")
//!     .header("Authorization", "Bearer …")
//!     .tools()?;
//! # Ok(()) }
//! ```
//!
//! | method | effect | idempotent | confirmation |
//! |--------|--------|------------|--------------|
//! | GET, HEAD | `read` | yes | no |
//! | PUT | `external_write` | yes | no |
//! | POST, PATCH | `external_write` | no | no |
//! | DELETE | `destructive` | yes | required |
//!
//! `x-carmy-effect` on an operation, or [`OpenApi::effect`], overrides the method's.
//! A call's `request_id` is sent as `Idempotency-Key` on writes. The input is an object
//! with the path and query parameters, and `body` when the operation takes one.
//! Responses are not validated against the description by default: APIs drift from
//! their documents, and a drifted field should not fail a call that succeeded. Opt in
//! with [`OpenApi::strict_output`].
use carmy_core::{
    AgentContext, AgentError, AgentResult, Confirmation, Effect, ErrorCategory, Tool, ToolMetadata,
};
use serde_json::{Map, Value, json};
use std::{collections::HashMap, fmt, sync::Arc};

/// Why a description could not become tools.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// Not JSON, or not an OpenAPI 3 description.
    Invalid(String),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Invalid(e) => write!(f, "invalid OpenAPI description: {e}"),
        }
    }
}
impl std::error::Error for Error {}

/// An OpenAPI description and how to reach the API it describes.
pub struct OpenApi {
    spec: Value,
    base_url: Option<String>,
    headers: Vec<(String, String)>,
    effects: HashMap<String, Effect>,
    only: Option<Vec<String>>,
    prefix: String,
    strict_output: bool,
    client: Option<reqwest::Client>,
}

impl OpenApi {
    /// A JSON description (OpenAPI 3.0 or 3.1).
    pub fn from_json(text: &str) -> Result<Self, Error> {
        let spec: Value = serde_json::from_str(text).map_err(|e| Error::Invalid(e.to_string()))?;
        if !spec["openapi"].as_str().is_some_and(|v| v.starts_with('3')) {
            return Err(Error::Invalid("expected `openapi: 3.x`".into()));
        }
        Ok(Self {
            spec,
            base_url: None,
            headers: Vec::new(),
            effects: HashMap::new(),
            only: None,
            prefix: String::new(),
            strict_output: false,
            client: None,
        })
    }
    pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self, Error> {
        Self::from_json(&std::fs::read_to_string(path).map_err(Error::Io)?)
    }
    /// Where the API lives. Defaults to the description's first `servers` URL.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }
    /// A header sent on every call, such as credentials. Never shown to agents.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
    /// Override the effect of one operation, by `operationId`.
    pub fn effect(mut self, operation_id: impl Into<String>, effect: Effect) -> Self {
        self.effects.insert(operation_id.into(), effect);
        self
    }
    /// Only these operations become tools, by `operationId`.
    pub fn only(mut self, operation_ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.only = Some(operation_ids.into_iter().map(Into::into).collect());
        self
    }
    /// Prepended to every tool name, e.g. `github_`.
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }
    /// Validate responses against the description's 2xx schemas.
    pub fn strict_output(mut self, strict: bool) -> Self {
        self.strict_output = strict;
        self
    }
    /// Use this HTTP client (proxies, timeouts, TLS settings).
    pub fn client(mut self, client: reqwest::Client) -> Self {
        self.client = Some(client);
        self
    }

    /// One tool per operation.
    pub fn tools(self) -> Result<Vec<OpenApiTool>, Error> {
        let base_url = match &self.base_url {
            Some(url) => url.clone(),
            None => self.spec["servers"][0]["url"]
                .as_str()
                .ok_or_else(|| Error::Invalid("no base_url and no `servers`".into()))?
                .to_owned(),
        };
        let http = Arc::new(Http {
            client: self.client.clone().unwrap_or_default(),
            base_url: base_url.trim_end_matches('/').to_owned(),
            headers: self.headers.clone(),
        });
        let paths = self.spec["paths"]
            .as_object()
            .ok_or_else(|| Error::Invalid("no `paths`".into()))?;
        let mut tools = Vec::new();
        for (path, item) in paths {
            let shared = item["parameters"].as_array().cloned().unwrap_or_default();
            for method in ["get", "head", "put", "post", "patch", "delete"] {
                let Some(operation) = item.get(method) else {
                    continue;
                };
                let id = operation["operationId"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{method}_{path}"));
                if let Some(only) = &self.only
                    && !only.contains(&id)
                {
                    continue;
                }
                tools.push(self.operation(&http, path, method, &id, operation, &shared)?);
            }
        }
        Ok(tools)
    }

    fn operation(
        &self,
        http: &Arc<Http>,
        path: &str,
        method: &str,
        id: &str,
        operation: &Value,
        shared: &[Value],
    ) -> Result<OpenApiTool, Error> {
        let mut properties = Map::new();
        let mut required = Vec::new();
        let mut params = Vec::new();
        let mut used = Vec::new();
        for parameter in shared
            .iter()
            .chain(operation["parameters"].as_array().into_iter().flatten())
        {
            let parameter = self.resolve(parameter);
            let (Some(name), Some(location)) =
                (parameter["name"].as_str(), parameter["in"].as_str())
            else {
                continue;
            };
            let location = match location {
                "path" => Location::Path,
                "query" => Location::Query,
                // Headers and cookies carry credentials and plumbing; the host sets them.
                _ => continue,
            };
            let mut schema = parameter.get("schema").cloned().unwrap_or(json!({}));
            if let Some(description) = parameter.get("description") {
                schema["description"] = description.clone();
            }
            collect_refs(&schema, &mut used);
            properties.insert(name.to_owned(), schema);
            if location == Location::Path || parameter["required"] == true {
                required.push(Value::String(name.to_owned()));
            }
            params.retain(|(n, _): &(String, Location)| n != name);
            params.push((name.to_owned(), location));
        }
        let body = operation.get("requestBody").map(|b| self.resolve(b));
        let has_body = body.is_some();
        if let Some(body) = &body {
            let schema = body["content"]["application/json"]["schema"].clone();
            let schema = if schema.is_null() { json!({}) } else { schema };
            collect_refs(&schema, &mut used);
            properties.insert("body".into(), schema);
            if body["required"] == true {
                required.push("body".into());
            }
        }
        let mut input = json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        });
        let mut output = json!({});
        if self.strict_output {
            let responses = operation["responses"].as_object();
            if let Some((_, response)) = responses
                .into_iter()
                .flatten()
                .find(|(code, _)| code.starts_with('2'))
            {
                let schema = &self.resolve(response)["content"]["application/json"]["schema"];
                if !schema.is_null() {
                    output = schema.clone();
                    collect_refs(&output, &mut used);
                }
            }
        }
        let defs = self.definitions(used);
        for schema in [&mut input, &mut output] {
            rewrite_refs(schema);
            upgrade_nullable(schema);
            if !defs.is_empty() && schema.as_object().is_some_and(|o| !o.is_empty()) {
                schema["$defs"] = Value::Object(defs.clone());
            }
        }

        let declared = operation["x-carmy-effect"].as_str().and_then(parse_effect);
        let effect = self
            .effects
            .get(id)
            .copied()
            .or(declared)
            .unwrap_or(match method {
                "get" | "head" => Effect::Read,
                "delete" => Effect::Destructive,
                _ => Effect::ExternalWrite,
            });
        let description = [
            operation["summary"].as_str(),
            operation["description"].as_str(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(". ");
        Ok(OpenApiTool {
            metadata: ToolMetadata {
                name: tool_name(&self.prefix, id),
                description: if description.is_empty() {
                    format!("{} {path}", method.to_uppercase())
                } else {
                    description
                },
                input_schema: input,
                output_schema: output,
                effect,
                idempotent: matches!(method, "get" | "head" | "put" | "delete"),
                parallel_safe: true,
                confirmation: if effect == Effect::Destructive {
                    Confirmation::Required
                } else {
                    Confirmation::None
                },
            },
            method: method.to_uppercase(),
            path: path.to_owned(),
            params,
            has_body,
            http: http.clone(),
        })
    }

    /// Follow a local `$ref` to a component (parameters, request bodies, responses).
    fn resolve(&self, value: &Value) -> Value {
        match value["$ref"].as_str().and_then(|r| r.strip_prefix('#')) {
            Some(pointer) => self.spec.pointer(pointer).cloned().unwrap_or(Value::Null),
            None => value.clone(),
        }
    }

    /// The component schemas `used` needs, transitively, keyed by name.
    fn definitions(&self, mut pending: Vec<String>) -> Map<String, Value> {
        let schemas = &self.spec["components"]["schemas"];
        let mut defs = Map::new();
        while let Some(name) = pending.pop() {
            if defs.contains_key(&name) {
                continue;
            }
            let Some(schema) = schemas.get(&name) else {
                continue;
            };
            collect_refs(schema, &mut pending);
            let mut schema = schema.clone();
            rewrite_refs(&mut schema);
            upgrade_nullable(&mut schema);
            defs.insert(name, schema);
        }
        defs
    }
}

const SCHEMA_REF: &str = "#/components/schemas/";

fn collect_refs(value: &Value, into: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(name) = map
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|r| r.strip_prefix(SCHEMA_REF))
            {
                into.push(name.to_owned());
            }
            map.values().for_each(|v| collect_refs(v, into));
        }
        Value::Array(items) => items.iter().for_each(|v| collect_refs(v, into)),
        _ => {}
    }
}

/// Component references point into the tool's own `$defs`, so each schema stands alone.
fn rewrite_refs(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get_mut("$ref")
                && let Some(name) = reference.strip_prefix(SCHEMA_REF)
            {
                *reference = format!("#/$defs/{name}");
            }
            map.values_mut().for_each(rewrite_refs);
        }
        Value::Array(items) => items.iter_mut().for_each(rewrite_refs),
        _ => {}
    }
}

/// OpenAPI 3.0's `nullable: true` becomes a JSON Schema type that admits `null`.
fn upgrade_nullable(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map.remove("nullable") == Some(Value::Bool(true))
                && let Some(Value::String(kind)) = map.get("type").cloned()
            {
                map.insert("type".into(), json!([kind, "null"]));
            }
            map.values_mut().for_each(upgrade_nullable);
        }
        Value::Array(items) => items.iter_mut().for_each(upgrade_nullable),
        _ => {}
    }
}

fn parse_effect(effect: &str) -> Option<Effect> {
    serde_json::from_value(Value::String(effect.to_owned())).ok()
}

/// Names Carmy accepts: ASCII letters, digits, `_`, `-`, `.`; at most 128 bytes.
fn tool_name(prefix: &str, id: &str) -> String {
    let name: String = format!("{prefix}{id}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "_-.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let name = name.trim_matches('_').to_owned();
    name.chars().take(128).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Location {
    Path,
    Query,
}

struct Http {
    client: reqwest::Client,
    base_url: String,
    headers: Vec<(String, String)>,
}

/// One API operation as a tool.
pub struct OpenApiTool {
    metadata: ToolMetadata,
    method: String,
    path: String,
    params: Vec<(String, Location)>,
    has_body: bool,
    http: Arc<Http>,
}

impl Tool for OpenApiTool {
    type Input = Value;
    type Output = Value;
    fn metadata(&self) -> ToolMetadata {
        self.metadata.clone()
    }
    async fn execute(&self, ctx: AgentContext, input: Value) -> AgentResult<Value> {
        let mut path = self.path.clone();
        let mut query = Vec::new();
        for (name, location) in &self.params {
            let Some(value) = input.get(name) else {
                continue;
            };
            let text = match value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            match location {
                // Encoded, `/` included, so a value can never leave its path segment.
                Location::Path => {
                    path = path.replace(&format!("{{{name}}}"), &encode_segment(&text));
                }
                Location::Query => match value {
                    Value::Array(items) => query.extend(items.iter().map(|item| {
                        (
                            name.clone(),
                            item.as_str()
                                .map_or_else(|| item.to_string(), str::to_owned),
                        )
                    })),
                    _ => query.push((name.clone(), text)),
                },
            }
        }
        let method = reqwest::Method::from_bytes(self.method.as_bytes()).expect("a known method");
        let mut request = self
            .http
            .client
            .request(method.clone(), format!("{}{path}", self.http.base_url))
            .query(&query)
            .header("accept", "application/json");
        for (name, value) in &self.http.headers {
            request = request.header(name, value);
        }
        // The call's identity travels with writes, so the API can dedupe retries too.
        if let Some(request_id) = &ctx.request_id
            && method != reqwest::Method::GET
            && method != reqwest::Method::HEAD
        {
            request = request.header("Idempotency-Key", request_id);
        }
        if self.has_body
            && let Some(body) = input.get("body")
        {
            request = request.json(body);
        }
        let response = request.send().await.map_err(|e| {
            if e.is_connect() {
                AgentError::new(
                    "UPSTREAM_UNREACHABLE",
                    e.to_string(),
                    ErrorCategory::Capacity,
                )
                .retryable(Some(1))
            } else {
                // Sent, but no answer: the API may have acted on it.
                AgentError::new(
                    "UPSTREAM_FAILED",
                    format!("{e}; the request may have reached the API"),
                    ErrorCategory::Internal,
                )
            }
        })?;
        let status = response.status();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        let text = response.text().await.map_err(|e| {
            AgentError::new("UPSTREAM_FAILED", e.to_string(), ErrorCategory::Internal)
        })?;
        let body = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or(Value::String(text))
        };
        if status.is_success() {
            return Ok(body);
        }
        Err(upstream_error(status.as_u16(), retry_after, body))
    }
}

/// An API's error answer as a Carmy error: the category an agent can act on, the
/// status and the body in `details`.
fn upstream_error(status: u16, retry_after: Option<u64>, body: Value) -> AgentError {
    let (code, category) = match status {
        401 | 403 => ("UPSTREAM_UNAUTHORIZED", ErrorCategory::Permission),
        404 => ("UPSTREAM_NOT_FOUND", ErrorCategory::NotFound),
        409 => ("UPSTREAM_CONFLICT", ErrorCategory::Conflict),
        429 => ("UPSTREAM_RATE_LIMITED", ErrorCategory::Capacity),
        500..=599 => ("UPSTREAM_UNAVAILABLE", ErrorCategory::Capacity),
        _ => ("UPSTREAM_REJECTED", ErrorCategory::Validation),
    };
    let mut body = body;
    if let Value::String(s) = &mut body
        && s.len() > 2048
    {
        s.truncate(2048);
    }
    let error = AgentError::new(code, format!("the API answered {status}"), category)
        .details(json!({ "status": status, "body": body }));
    match status {
        429 | 503 => error.retryable(retry_after.or(Some(1))),
        500..=599 => error.retryable(None),
        _ => error,
    }
}

fn encode_segment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_segments_are_safe() {
        assert_eq!(
            tool_name("gh_", "repos/get content"),
            "gh_repos_get_content"
        );
        assert_eq!(encode_segment("a/../b c"), "a%2F..%2Fb%20c");
    }

    #[test]
    fn nullable_becomes_a_type_union() {
        let mut schema =
            json!({"type": "object", "properties": {"n": {"type": "string", "nullable": true}}});
        upgrade_nullable(&mut schema);
        assert_eq!(schema["properties"]["n"]["type"], json!(["string", "null"]));
    }

    #[test]
    fn upstream_errors_map_to_categories() {
        let limited = upstream_error(429, Some(7), json!({}));
        assert_eq!(limited.code, "UPSTREAM_RATE_LIMITED");
        assert!(limited.retryable);
        assert_eq!(limited.retry_after, Some(7));
        assert!(upstream_error(502, None, Value::Null).retryable);
        assert!(!upstream_error(400, None, Value::Null).retryable);
        assert_eq!(
            upstream_error(404, None, Value::Null).category,
            ErrorCategory::NotFound
        );
    }
}
