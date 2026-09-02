//! Connection to the single upstream MCP server, spawned as a subprocess
//! and spoken to over stdio with raw JSON-RPC messages.
//!
//! Unlike `arkavo_mcp_runtime::McpClient`, this connection performs no
//! handshake of its own and returns the upstream response object verbatim,
//! so the proxy can relay the downstream client's `initialize` and preserve
//! upstream error codes and messages exactly.
//!
//! Traffic flows one way: the downstream client asks, the upstream server
//! answers. A server-initiated request — `sampling/createMessage`,
//! `elicitation/create`, `roots/list` — is **not** relayed to the client in
//! this slice, because the client's permit and proof cover the call it made
//! and nothing the server thinks of afterwards. Such a request is answered
//! here with JSON-RPC `-32601`, so the upstream learns immediately instead of
//! blocking until its own timeout. Server notifications, which expect no
//! answer, are logged and dropped.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, oneshot};
use tracing::{debug, warn};

/// Errors from the upstream connection.
#[derive(Debug, thiserror::Error)]
pub enum UpstreamError {
    /// The upstream server process could not be spawned.
    #[error("failed to spawn upstream server '{command}': {source}")]
    Spawn {
        /// Command that failed to spawn.
        command: String,
        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// The upstream server closed its stdout or never responded.
    #[error("upstream connection closed")]
    Closed,

    /// Writing to the upstream server's stdin failed.
    #[error("upstream write failed: {0}")]
    Write(String),

    /// The upstream server did not respond in time.
    #[error("upstream request timed out after {0:?}")]
    Timeout(Duration),
}

/// Default per-request timeout when the caller does not configure one.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// JSON-RPC method-not-found, the answer to a server-initiated request this
/// slice does not relay to the downstream client.
const METHOD_NOT_FOUND: i64 = -32601;

/// Key used to correlate responses with pending requests. JSON-RPC allows
/// string or numeric ids; the serialized form is a stable key for both.
fn id_key(id: &Value) -> String {
    serde_json::to_string(id).unwrap_or_else(|_| "null".to_string())
}

/// A spawned upstream MCP server reached over stdio.
pub struct UpstreamConnection {
    child: Mutex<Child>,
    /// Shared with the reader task, which answers server-initiated requests.
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    connected: Arc<AtomicBool>,
    timeout: Duration,
}

impl UpstreamConnection {
    /// Spawn `command args` and connect to its stdio.
    pub fn spawn(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        timeout: Option<Duration>,
    ) -> Result<Self, UpstreamError> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        for (key, value) in env {
            cmd.env(key, value);
        }

        let mut child = cmd.spawn().map_err(|source| UpstreamError::Spawn {
            command: command.to_string(),
            source,
        })?;

        let stdin = Arc::new(Mutex::new(child.stdin.take().ok_or(UpstreamError::Closed)?));
        let stdout = child.stdout.take().ok_or(UpstreamError::Closed)?;

        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let connected = Arc::new(AtomicBool::new(true));

        // Reader task: correlate responses by id, fail all pending requests
        // when the upstream closes its stdout.
        let reader_pending = Arc::clone(&pending);
        let reader_connected = Arc::clone(&connected);
        let reader_stdin = Arc::clone(&stdin);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => match serde_json::from_str::<Value>(&line) {
                        Ok(message) => {
                            if let Some(id) = message.get("id").filter(|v| !v.is_null()).cloned() {
                                let sender = reader_pending.lock().await.remove(&id_key(&id));
                                match sender {
                                    Some(sender) => {
                                        let _ = sender.send(message);
                                    }
                                    // An id nothing is waiting on. If it
                                    // names a method it is a request the
                                    // server made of us; answering it is what
                                    // keeps the server from blocking on a
                                    // reply this slice will never send.
                                    None => {
                                        refuse_server_request(&reader_stdin, &id, &message).await;
                                    }
                                }
                            } else {
                                debug!("upstream notification (dropped): {line}");
                            }
                        }
                        Err(e) => warn!("unparseable upstream output: {e}"),
                    },
                    Ok(None) => break,
                    Err(e) => {
                        warn!("upstream read failed: {e}");
                        break;
                    }
                }
            }
            reader_connected.store(false, Ordering::SeqCst);
            // Dropping the senders resolves every pending receiver with an
            // error instead of leaving callers to wait out the timeout.
            reader_pending.lock().await.clear();
        });

        Ok(Self {
            child: Mutex::new(child),
            stdin,
            pending,
            connected,
            timeout: timeout.unwrap_or(DEFAULT_TIMEOUT),
        })
    }

    /// Send a request and return the full JSON-RPC response object,
    /// including any upstream error, verbatim.
    pub async fn request(
        &self,
        id: &Value,
        method: &str,
        params: Option<&Value>,
    ) -> Result<Value, UpstreamError> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(UpstreamError::Closed);
        }

        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params.cloned().unwrap_or_else(|| json!({})),
        });
        let key = id_key(id);

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(key.clone(), tx);
        // Re-check after inserting: the reader task may have observed
        // upstream EOF between the first check and the insert and already
        // cleared pending — without this the request would hang until the
        // timeout instead of failing fast.
        if !self.connected.load(Ordering::SeqCst) {
            self.pending.lock().await.remove(&key);
            return Err(UpstreamError::Closed);
        }
        if let Err(e) = write_line(&self.stdin, &request).await {
            self.pending.lock().await.remove(&key);
            return Err(e);
        }

        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(UpstreamError::Closed),
            Err(_) => {
                self.pending.lock().await.remove(&key);
                Err(UpstreamError::Timeout(self.timeout))
            }
        }
    }

    /// Send a notification; no response is expected.
    pub async fn notify(&self, method: &str, params: Option<&Value>) -> Result<(), UpstreamError> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(UpstreamError::Closed);
        }
        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params.cloned().unwrap_or_else(|| json!({})),
        });
        write_line(&self.stdin, &notification).await
    }

    /// Terminate the upstream server process.
    pub async fn shutdown(&self) {
        self.connected.store(false, Ordering::SeqCst);
        if let Err(e) = self.child.lock().await.kill().await {
            debug!("upstream process already exited: {e}");
        }
    }
}

/// Answer a request the upstream server made of us.
///
/// A message with an id that matches no pending request is either a stray
/// response — nothing to do about that but say so — or a server-initiated
/// request. The proxy does not relay those to the downstream client, so it
/// refuses them here rather than letting the server wait out its own timeout.
async fn refuse_server_request(
    stdin: &Mutex<tokio::process::ChildStdin>,
    id: &Value,
    message: &Value,
) {
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        warn!("upstream response with an id nothing is waiting on (dropped)");
        return;
    };
    warn!(
        method,
        "refusing a server-initiated request: not relayed to the downstream client"
    );
    let refusal = json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": METHOD_NOT_FOUND,
            "message": format!(
                "this proxy does not relay server-initiated requests to the client ({method})"
            ),
        },
    });
    if let Err(e) = write_line(stdin, &refusal).await {
        warn!("failed to refuse server-initiated request '{method}': {e}");
    }
}

async fn write_line(
    stdin: &Mutex<tokio::process::ChildStdin>,
    message: &Value,
) -> Result<(), UpstreamError> {
    let mut bytes = serde_json::to_vec(message).unwrap_or_default();
    bytes.push(b'\n');
    let mut stdin = stdin.lock().await;
    stdin
        .write_all(&bytes)
        .await
        .map_err(|e| UpstreamError::Write(e.to_string()))?;
    stdin
        .flush()
        .await
        .map_err(|e| UpstreamError::Write(e.to_string()))
}

impl std::fmt::Debug for UpstreamConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpstreamConnection")
            .field("connected", &self.connected.load(Ordering::SeqCst))
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
// The `#[tokio::test]` macro expands to `Runtime::block_on`, which
// `.clippy.toml` disallows outside test code.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Regression: after the upstream exits, a request must fail fast with
    /// `Closed` — never hang for the full timeout because the reader task
    /// cleared `pending` before the request was inserted.
    #[tokio::test]
    async fn request_fails_closed_fast_after_upstream_exit() {
        // `true` exits immediately without reading or writing anything.
        let conn = UpstreamConnection::spawn("true", &[], &HashMap::new(), None).unwrap();

        // Wait for the reader task to observe EOF and mark disconnected.
        for _ in 0..200 {
            if !conn.connected.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            !conn.connected.load(Ordering::SeqCst),
            "reader task must mark the connection closed after upstream EOF"
        );

        let start = Instant::now();
        let err = conn
            .request(&json!(1), "tools/call", None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, UpstreamError::Closed),
            "request after upstream exit must fail with Closed, got {err}"
        );
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "request after upstream exit must fail fast, not wait out the timeout"
        );
    }
}
