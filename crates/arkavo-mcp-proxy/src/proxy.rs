//! Stdio pass-through MCP proxy with per-call policy enforcement.

use crate::framing::{self, Line, MAX_LINE_BYTES};
use crate::policy::{CallContext, Credential, Decision, ForwardOutcome, PolicyHook};
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

/// The longest base64url string `_meta.arkavo.permit` may carry: the permit
/// size cap re-expressed in encoded characters (four per three bytes, plus a
/// partial group). Anything longer cannot decode to a permit this stack would
/// accept, so it is refused without decoding it.
const MAX_ENCODED_PERMIT: usize = 4 * arkavo_dispatch_gate::MAX_PERMIT_BYTES / 3 + 4;

/// The longest base64url string `_meta.arkavo.pop` may carry.
///
/// A proof of possession is one signature — 64 bytes from both key types
/// this stack signs with, Ed25519 and P-256 in P1363 form — which is 86
/// characters unpadded. Bounding it by the permit's cap instead would let a
/// caller send 21 849 characters of base64 for something that can only ever
/// be 86, and hold the difference in memory while it was decoded.
const MAX_ENCODED_PROOF: usize = 88;

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

/// Write one framed response to the downstream client.
async fn write_response<W: AsyncWrite + Unpin>(
    writer: &mut W,
    bytes: &[u8],
) -> std::io::Result<()> {
    writer.write_all(bytes).await?;
    writer.flush().await
}

/// Whether a downstream write failed because the client is no longer there.
///
/// A client is free to stop listening at any point, and it does not owe the
/// proxy a clean shutdown: the case that reaches here is a client that sends
/// an over-long line — which is answered, not ignored — and closes the
/// connection before the answer can be written.
fn client_is_gone(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
    )
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
                if let Err(error) = write_response(&mut writer, &bytes).await {
                    if !client_is_gone(&error) {
                        return Err(error.into());
                    }
                    // The client left before its answer could be written.
                    // That is how a session ends — a client that sends an
                    // over-long line and disconnects reaches exactly here —
                    // and reporting it as a failure of the proxy would be
                    // reporting someone else's hang-up.
                    debug!("downstream client disconnected before the response was written");
                    break;
                }
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
        let permit = credential(meta, "permit", MAX_ENCODED_PERMIT);
        let proof = credential(meta, "pop", MAX_ENCODED_PROOF);
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
                        // No response came back. Whether whatever the policy
                        // spent admitting the call can be handed back turns
                        // on how far the call got: a request that was written
                        // upstream may be running there still, a timeout
                        // being the ordinary way to see that. An error
                        // returned *by the tool* is a completed call and does
                        // not come through here at all.
                        let outcome = ForwardOutcome::from(&error);
                        self.policy.on_forward_failed(&ctx, outcome).await;
                        warn!(
                            tool = %ctx.tool_name,
                            error = %error,
                            ?outcome,
                            "tool call produced no upstream response"
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
///
/// `max_encoded` is this field's own bound: a permit and a proof of
/// possession differ by three orders of magnitude in what they can
/// legitimately be, so one cap for both is barely a cap on the proof.
fn credential(meta: Option<&Value>, key: &str, max_encoded: usize) -> Credential {
    match meta.and_then(|m| m.get(key)) {
        None => Credential::Absent,
        Some(value) => match value.as_str() {
            // A non-string is as unusable as a malformed string, and saying
            // so is more use to the client than calling it absent.
            None => Credential::Undecodable,
            Some(text) if text.len() > max_encoded => Credential::Oversized,
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
    /// field is too long to be what it claims to be.
    #[test]
    #[spec("PDG-009")]
    fn credential_distinguishes_absent_undecodable_and_oversized() {
        let meta = json!({
            "permit": "aGk",
            "pop": "!!! not base64",
            "huge": "A".repeat(MAX_ENCODED_PERMIT + 1),
            "number": 7,
        });
        let meta = Some(&meta);

        assert_eq!(
            credential(meta, "permit", MAX_ENCODED_PERMIT),
            Credential::Present(b"hi".to_vec())
        );
        assert_eq!(
            credential(meta, "pop", MAX_ENCODED_PROOF),
            Credential::Undecodable
        );
        assert_eq!(
            credential(meta, "huge", MAX_ENCODED_PERMIT),
            Credential::Oversized
        );
        assert_eq!(
            credential(meta, "number", MAX_ENCODED_PERMIT),
            Credential::Undecodable
        );
        assert_eq!(
            credential(meta, "missing", MAX_ENCODED_PERMIT),
            Credential::Absent
        );
        assert_eq!(
            credential(None, "permit", MAX_ENCODED_PERMIT),
            Credential::Absent
        );
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
    /// the credential it claims to be, not what merely approaches its size.
    #[test]
    fn a_credential_at_the_encoded_cap_is_not_oversized() {
        let meta = json!({
            "permit": "A".repeat(MAX_ENCODED_PERMIT),
            "pop": "A".repeat(MAX_ENCODED_PROOF),
        });
        let meta = Some(&meta);
        assert_ne!(
            credential(meta, "permit", MAX_ENCODED_PERMIT),
            Credential::Oversized
        );
        assert_ne!(
            credential(meta, "pop", MAX_ENCODED_PROOF),
            Credential::Oversized
        );
    }

    /// The proof's own bound is what makes it a bound at all: a 64-byte
    /// signature is 86 characters, and everything from there to the permit's
    /// 21 849 was accepted while the two shared one cap.
    #[test]
    #[spec("PDG-009")]
    fn a_proof_longer_than_a_signature_is_oversized() {
        let real_proof = "A".repeat(86);
        assert!(real_proof.len() <= MAX_ENCODED_PROOF);

        let meta = json!({
            "pop": "A".repeat(MAX_ENCODED_PROOF + 1),
            "permit": "A".repeat(MAX_ENCODED_PROOF + 1),
        });
        let meta = Some(&meta);
        assert_eq!(
            credential(meta, "pop", MAX_ENCODED_PROOF),
            Credential::Oversized
        );
        // The same string is nowhere near the permit's cap, which is the
        // point: one cap for both fields left the proof unbounded in practice.
        assert_ne!(
            credential(meta, "permit", MAX_ENCODED_PERMIT),
            Credential::Oversized
        );
    }
}
