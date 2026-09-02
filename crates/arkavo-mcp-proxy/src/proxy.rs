//! Stdio pass-through MCP proxy with per-call policy enforcement.

use crate::policy::{CallContext, Decision, PolicyHook};
use crate::upstream::{UpstreamConnection, UpstreamError};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
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
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).await? == 0 {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(response) = self.handle_message(trimmed).await {
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
        let permit = meta
            .and_then(|m| m.get("permit"))
            .and_then(Value::as_str)
            .and_then(decode_b64url);
        let proof = meta
            .and_then(|m| m.get("pop"))
            .and_then(Value::as_str)
            .and_then(decode_b64url);
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
                let response = self
                    .forward(id, "tools/call", forwarded_params.as_ref())
                    .await;
                info!(
                    tool = %ctx.tool_name,
                    decision = "allow",
                    latency_ms = started.elapsed().as_millis(),
                    "tool call forwarded"
                );
                response
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
mod tests {
    use super::*;

    #[test]
    fn error_response_shape() {
        let resp = error_response(json!(7), POLICY_DENIED, "nope".to_string());
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 7);
        assert_eq!(resp["error"]["code"], -32000);
        assert_eq!(resp["error"]["message"], "nope");
        assert!(resp.get("result").is_none());
    }
}
