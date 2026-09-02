//! Stdio pass-through MCP proxy with per-call policy enforcement.

use crate::framing::{self, Line, MAX_LINE_BYTES};
use crate::policy::{CallContext, Credential, Decision, PolicyHook};
use crate::upstream::{UpstreamConnection, UpstreamError};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufRead, AsyncWrite, AsyncWriteExt};
use tracing::{debug, info, warn};

/// JSON-RPC parse error (invalid JSON received).
pub const PARSE_ERROR: i64 = -32700;
/// JSON-RPC invalid request (missing method or malformed envelope).
pub const INVALID_REQUEST: i64 = -32600;
/// Server error: the policy hook denied the call.
pub const POLICY_DENIED: i64 = -32000;
/// Server error: the upstream connection failed.
pub const UPSTREAM_ERROR: i64 = -32603;

/// The longest base64url string `_meta.arkavo` may carry: the permit size cap
/// re-expressed in encoded characters (four per three bytes, plus a partial
/// group). Anything longer cannot decode to a permit this stack would accept,
/// so it is refused without decoding it.
const MAX_ENCODED_CREDENTIAL: usize = 4 * arkavo_dispatch_gate::MAX_PERMIT_BYTES / 3 + 4;

/// Configuration for connecting to the upstream MCP server.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Command used to spawn the upstream MCP server.
    pub command: String,
    /// Arguments for the upstream command.
    pub args: Vec<String>,
    /// Extra environment variables passed to the upstream process.
    pub env: HashMap<String, String>,
    /// Per-request timeout for upstream calls; `None` uses the default.
    pub request_timeout: Option<Duration>,
}

impl ProxyConfig {
    /// Create a config that spawns `command args` as the upstream server.
    pub fn new(command: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            command: command.into(),
            args,
            env: HashMap::new(),
            request_timeout: None,
        }
    }

    /// Add an environment variable for the upstream process.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Set the per-request upstream timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = Some(timeout);
        self
    }
}

/// Errors that can terminate the proxy loop.
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    /// Upstream connection failure.
    #[error("upstream error: {0}")]
    Upstream(#[from] UpstreamError),

    /// Downstream stdio failure.
    #[error("downstream I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A response could not be serialized.
    #[error("response serialization failed: {0}")]
    Json(#[from] serde_json::Error),
}

/// Build a JSON-RPC error response object.
fn error_response(id: Value, code: i64, message: String) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
}

/// MCP interception proxy serving one downstream client on stdio and
/// forwarding to one upstream MCP server subprocess.
pub struct McpProxy {
    upstream: UpstreamConnection,
    policy: Arc<dyn PolicyHook>,
}

impl McpProxy {
    /// Spawn the upstream server and return a ready proxy.
    pub fn spawn(config: ProxyConfig, policy: Arc<dyn PolicyHook>) -> Result<Self, UpstreamError> {
        let upstream = UpstreamConnection::spawn(
            &config.command,
            &config.args,
            &config.env,
            config.request_timeout,
        )?;
        Ok(Self { upstream, policy })
    }

    /// Serve JSON-RPC requests from `reader` and write responses to
    /// `writer` until EOF, then shut the upstream server down.
    ///
    /// Production use passes `BufReader::new(tokio::io::stdin())` and
    /// `tokio::io::stdout()`; tests can pass in-memory streams.
    pub async fn run<R, W>(&self, mut reader: R, mut writer: W) -> Result<(), ProxyError>
    where
        R: AsyncBufRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        loop {
            let response = match framing::read_line(&mut reader).await? {
                Line::Eof => break,
                Line::TooLong => {
                    warn!(
                        max_bytes = MAX_LINE_BYTES,
                        "dropped an over-long message; no id could be read from it, so the \
                         error carries a null id"
                    );
                    Some(error_response(
                        Value::Null,
                        INVALID_REQUEST,
                        format!("message exceeds the {MAX_LINE_BYTES} byte limit"),
                    ))
                }
                Line::Message(line) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    self.handle_message(trimmed).await
                }
            };
            if let Some(response) = response {
                let mut bytes = serde_json::to_vec(&response)?;
                bytes.push(b'\n');
                writer.write_all(&bytes).await?;
                writer.flush().await?;
            }
        }
        self.upstream.shutdown().await;
        Ok(())
    }

    /// Handle one downstream message; returns the response for requests and
    /// `None` for notifications.
    async fn handle_message(&self, text: &str) -> Option<Value> {
        let message = match serde_json::from_str::<Value>(text) {
            Ok(message) => message,
            Err(e) => {
                return Some(error_response(
                    Value::Null,
                    PARSE_ERROR,
                    format!("parse error: {e}"),
                ));
            }
        };

        // A JSON-RPC batch. Every message in one would have to be gated
        // individually and answered in a single array, which this slice does
        // not do; answering says so instead of leaving the client waiting.
        if message.is_array() {
            warn!("rejected a JSON-RPC batch: this proxy handles one message per line");
            return Some(error_response(
                Value::Null,
                INVALID_REQUEST,
                "JSON-RPC batches are not supported; send one request per line".to_string(),
            ));
        }

        let id = message.get("id").cloned().filter(|v| !v.is_null());
        let params = message.get("params");
        let method = message.get("method").and_then(Value::as_str);

        match (id, method) {
            (id, None) => {
                id.map(|id| error_response(id, INVALID_REQUEST, "missing method".to_string()))
            }
            (None, Some(method)) => {
                if method == "tools/call" {
                    // A `tools/call` with no `id` cannot be answered, so it
                    // cannot be policy-evaluated either: dropping it here
                    // (rather than forwarding it as a notification) is what
                    // keeps every `tools/call` gated, since an adversary
                    // could otherwise omit `id` to bypass the policy hook
                    // entirely.
                    let tool = params
                        .and_then(|p| p.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    warn!(
                        tool,
                        "dropped tools/call notification: calls must carry an id so policy can answer them"
                    );
                    return None;
                }
                // Notification: forward upstream, no response downstream.
                if let Err(e) = self.upstream.notify(method, params).await {
                    warn!("failed to forward notification '{method}': {e}");
                }
                None
            }
            (Some(id), Some(method)) => Some(self.handle_request(id, method, params).await),
        }
    }

    async fn handle_request(&self, id: Value, method: &str, params: Option<&Value>) -> Value {
        if method == "tools/call" {
            return self.handle_tool_call(id, params).await;
        }
        debug!("forwarding '{method}'");
        self.forward(id, method, params).await
    }

    async fn handle_tool_call(&self, id: Value, params: Option<&Value>) -> Value {
        let tool_name = params
            .and_then(|p| p.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let arguments = params
            .and_then(|p| p.get("arguments"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let meta = params
            .and_then(|p| p.get("_meta"))
            .and_then(|m| m.get("arkavo"));
        let permit = credential(meta, "permit");
        let proof = credential(meta, "pop");
        let ctx = CallContext {
            tool_name,
            arguments,
            permit,
            proof,
        };

        let started = Instant::now();
        let decision = self.policy.evaluate(&ctx).await;

        match decision {
            Decision::Deny { reason } => {
                info!(
                    tool = %ctx.tool_name,
                    decision = "deny",
                    reason = %reason,
                    latency_ms = started.elapsed().as_millis(),
                    "tool call blocked by policy"
                );
                error_response(
                    id,
                    POLICY_DENIED,
                    format!("tool '{}' denied by policy: {reason}", ctx.tool_name),
                )
            }
            Decision::Allow => {
                let forwarded_params = strip_arkavo_meta(params);
                match self
                    .upstream
                    .request(&id, "tools/call", forwarded_params.as_ref())
                    .await
                {
                    Ok(response) => {
                        info!(
                            tool = %ctx.tool_name,
                            decision = "allow",
                            latency_ms = started.elapsed().as_millis(),
                            "tool call forwarded"
                        );
                        response
                    }
                    Err(error) => {
                        // The upstream never ran the call, so whatever the
                        // policy spent admitting it is handed back. An error
                        // returned *by the tool* is a completed call and does
                        // not come through here.
                        self.policy.on_forward_failed(&ctx).await;
                        warn!(
                            tool = %ctx.tool_name,
                            error = %error,
                            "tool call never reached the upstream server"
                        );
                        error_response(id, UPSTREAM_ERROR, error.to_string())
                    }
                }
            }
        }
    }

    /// Forward a request upstream and relay its response verbatim; upstream
    /// transport failures become JSON-RPC error responses.
    async fn forward(&self, id: Value, method: &str, params: Option<&Value>) -> Value {
        match self.upstream.request(&id, method, params).await {
            Ok(response) => response,
            Err(e) => error_response(id, UPSTREAM_ERROR, e.to_string()),
        }
    }
}

impl std::fmt::Debug for McpProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpProxy")
            .field("upstream", &self.upstream)
            .finish_non_exhaustive()
    }
}

/// Read one `_meta.arkavo` credential, keeping *why* it is unusable.
fn credential(meta: Option<&Value>, key: &str) -> Credential {
    match meta.and_then(|m| m.get(key)) {
        None => Credential::Absent,
        Some(value) => match value.as_str() {
            // A non-string is as unusable as a malformed string, and saying
            // so is more use to the client than calling it absent.
            None => Credential::Undecodable,
            Some(text) if text.len() > MAX_ENCODED_CREDENTIAL => Credential::Oversized,
            Some(text) => decode_b64url(text).map_or(Credential::Undecodable, Credential::Present),
        },
    }
}

/// Decode a base64url-without-padding string, as used by `_meta.arkavo`.
fn decode_b64url(text: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text)
        .ok()
}

/// Strip the `arkavo` key out of `params._meta` before forwarding an
/// allowed call upstream, so the live permit and proof-of-possession never
/// leave the proxy. Every other `_meta` key travels unchanged; `_meta`
/// itself is dropped only if stripping `arkavo` leaves it empty.
fn strip_arkavo_meta(params: Option<&Value>) -> Option<Value> {
    let mut params = params?.clone();
    if let Some(object) = params.as_object_mut() {
        let empty = object
            .get_mut("_meta")
            .and_then(Value::as_object_mut)
            .map(|meta| {
                meta.remove("arkavo");
                meta.is_empty()
            });
        if empty == Some(true) {
            object.remove("_meta");
        }
    }
    Some(params)
}

#[cfg(test)]
// The `#[tokio::test]` macro expands to `Runtime::block_on`, which
// `.clippy.toml` disallows outside test code.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[test]
    fn error_response_shape() {
        let resp = error_response(json!(7), POLICY_DENIED, "nope".to_string());
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 7);
        assert_eq!(resp["error"]["code"], -32000);
        assert_eq!(resp["error"]["message"], "nope");
        assert!(resp.get("result").is_none());
    }

    #[test]
    fn decode_b64url_accepts_only_unpadded_base64url() {
        assert_eq!(decode_b64url("aGk").as_deref(), Some(&b"hi"[..]));
        assert_eq!(decode_b64url("").as_deref(), Some(&b""[..]));
        // Padding, the standard alphabet and stray characters are refused
        // rather than silently decoding to something else.
        assert_eq!(decode_b64url("aGk="), None);
        assert_eq!(decode_b64url("a+/b"), None);
        assert_eq!(decode_b64url("not base64!"), None);
    }

    /// The ways a credential can be unusable have to stay distinguishable: a
    /// client that sent nothing, one whose encoding is wrong, and one whose
    /// field is too long to be a permit at all.
    #[test]
    #[spec("PDG-009")]
    fn credential_distinguishes_absent_undecodable_and_oversized() {
        let meta = json!({
            "permit": "aGk",
            "pop": "!!! not base64",
            "huge": "A".repeat(MAX_ENCODED_CREDENTIAL + 1),
            "number": 7,
        });
        let meta = Some(&meta);

        assert_eq!(
            credential(meta, "permit"),
            Credential::Present(b"hi".to_vec())
        );
        assert_eq!(credential(meta, "pop"), Credential::Undecodable);
        assert_eq!(credential(meta, "huge"), Credential::Oversized);
        assert_eq!(credential(meta, "number"), Credential::Undecodable);
        assert_eq!(credential(meta, "missing"), Credential::Absent);
        assert_eq!(credential(None, "permit"), Credential::Absent);
    }

    /// The permit and proof are the proxy's own credentials and must not
    /// travel upstream, but the rest of `_meta` is the client's and must.
    #[test]
    #[spec("PDG-008")]
    fn strip_arkavo_meta_removes_only_the_arkavo_key() {
        let params = json!({
            "name": "echo",
            "arguments": {"n": 1},
            "_meta": {"arkavo": {"permit": "p", "pop": "q"}, "trace": "t-1"},
        });
        let stripped = strip_arkavo_meta(Some(&params)).expect("params");
        assert_eq!(stripped["_meta"], json!({"trace": "t-1"}));
        assert_eq!(stripped["arguments"], json!({"n": 1}));

        // `_meta` itself goes only when nothing else was in it.
        let only_arkavo = json!({"name": "echo", "_meta": {"arkavo": {"permit": "p"}}});
        let stripped = strip_arkavo_meta(Some(&only_arkavo)).expect("params");
        assert!(stripped.get("_meta").is_none(), "stripped: {stripped}");

        // Params without `_meta` are forwarded untouched, and a call with no
        // params at all stays that way.
        let bare = json!({"name": "echo"});
        assert_eq!(strip_arkavo_meta(Some(&bare)), Some(bare));
        assert_eq!(strip_arkavo_meta(None), None);
    }

    /// A `tools/call` with no id could never be answered, so it could never
    /// be answered with a denial either. It is dropped rather than forwarded,
    /// which is what keeps every `tools/call` gated.
    #[tokio::test]
    #[spec("PDG-007")]
    async fn a_tools_call_notification_is_dropped_not_forwarded() {
        use crate::policy::AllowAllPolicy;
        use std::sync::Arc;

        // `true` exits at once: nothing here should reach an upstream at all.
        let proxy = McpProxy::spawn(
            ProxyConfig::new("true", Vec::new()),
            Arc::new(AllowAllPolicy),
        )
        .expect("spawn");

        let notification = json!({"jsonrpc": "2.0", "method": "tools/call",
                                  "params": {"name": "echo", "arguments": {}}})
        .to_string();
        assert!(
            proxy.handle_message(&notification).await.is_none(),
            "a notification is never answered"
        );

        // A request, by contrast, is answered — here with the upstream error
        // that proves it was the id, not the method, that made the difference.
        let request = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                             "params": {"name": "echo", "arguments": {}}})
        .to_string();
        let response = proxy
            .handle_message(&request)
            .await
            .expect("a request is answered");
        assert_eq!(response["error"]["code"], UPSTREAM_ERROR);
    }

    /// A string at the cap is still decoded: the bound refuses what cannot be
    /// a permit, not what merely approaches the size of one.
    #[test]
    fn a_credential_at_the_encoded_cap_is_not_oversized() {
        let at_cap = "A".repeat(MAX_ENCODED_CREDENTIAL);
        let meta = json!({ "permit": at_cap });
        assert_ne!(credential(Some(&meta), "permit"), Credential::Oversized);
    }
}
