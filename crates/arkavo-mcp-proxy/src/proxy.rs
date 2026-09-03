//! Stdio pass-through MCP proxy with per-call policy enforcement.

use crate::framing::{self, Line, MAX_LINE_BYTES};
use crate::meta::{credentials, strip_arkavo_meta};
use crate::policy::{CallContext, Decision, ForwardOutcome, PolicyHook};
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

/// Whether a downstream read or write failed because the client is no longer
/// there.
///
/// A client is free to stop listening at any point, and it does not owe the
/// proxy a clean shutdown. Writing finds this when a client sends an
/// over-long line — which is answered, not ignored — and closes before the
/// answer can be written; reading finds it when the connection is reset
/// rather than closed, which is a hang-up too and not something the proxy
/// did. Neither is a failure of this session, so both end it with `Ok`.
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
            let line = match framing::read_line(&mut reader).await {
                Ok(line) => line,
                // A reset connection is a hang-up, not a fault: the client
                // stopped talking without saying so, which is the same end of
                // session an EOF is and is reported the same way.
                Err(error) if client_is_gone(&error) => {
                    debug!("downstream client disconnected mid-message");
                    break;
                }
                Err(error) => return Err(error.into()),
            };
            let response = match line {
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
        let (permit, proof) = credentials(params);
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
                        // No response came back. Timeouts and post-write
                        // failures never refund: the call may have executed.
                        // Only a request that was never written upstream is
                        // refunded. (An error returned *by the tool* is a
                        // completed call and does not come through here at
                        // all.)
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

    /// A downstream connection that fails on every read, with the error kind
    /// a test wants: a reset, or something the proxy really should report.
    struct FailingReader(std::io::ErrorKind);

    impl tokio::io::AsyncRead for FailingReader {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Err(std::io::Error::from(self.0)))
        }
    }

    /// A client that hangs up is a client that hangs up, whichever side of
    /// the connection notices it. A write that finds nobody there already
    /// ended the session with `Ok`; a *read* that finds the connection reset
    /// - the ordinary way a vanished client is noticed first - reported
    /// someone else's disconnect as a failure of the proxy.
    #[tokio::test]
    async fn a_client_whose_connection_is_reset_ends_the_session_cleanly() {
        use crate::policy::AllowAllPolicy;
        use std::sync::Arc;

        let proxy = McpProxy::spawn(
            ProxyConfig::new("true", Vec::new()),
            Arc::new(AllowAllPolicy),
        )
        .expect("spawn");

        for kind in [
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::BrokenPipe,
            std::io::ErrorKind::ConnectionAborted,
        ] {
            let result = proxy
                .run(
                    tokio::io::BufReader::new(FailingReader(kind)),
                    tokio::io::sink(),
                )
                .await;
            assert!(
                result.is_ok(),
                "{kind:?} is the client hanging up, not a failure of the proxy: {result:?}"
            );
        }

        // Every other read failure is still the proxy's to report: a session
        // that ends because the stream itself is unusable is not a hang-up.
        let result = proxy
            .run(
                tokio::io::BufReader::new(FailingReader(std::io::ErrorKind::InvalidData)),
                tokio::io::sink(),
            )
            .await;
        assert!(
            matches!(result, Err(ProxyError::Io(_))),
            "a read that is not a disconnect must still surface: {result:?}"
        );
    }
}
