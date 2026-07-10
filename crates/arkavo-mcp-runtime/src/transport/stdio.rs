use super::{Transport, TransportError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, oneshot};
use tracing::{debug, error, warn};

/// Stdio transport for MCP subprocess communication
pub struct StdioTransport {
    child: Arc<Mutex<Child>>,
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    request_id: AtomicU64,
    connected: AtomicBool,
    timeout: std::time::Duration,
}

impl StdioTransport {
    /// Create a new stdio transport by spawning a subprocess
    #[allow(clippy::unused_async)]
    pub async fn new(
        command: String,
        args: Vec<String>,
        cwd: Option<String>,
        env: HashMap<String, String>,
    ) -> Result<Self, TransportError> {
        let mut cmd = Command::new(&command);
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        for (key, value) in env {
            cmd.env(key, value);
        }

        let mut child = cmd.spawn().map_err(|e| {
            TransportError::ConnectionFailed(format!("Failed to spawn {command}: {e}"))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| TransportError::ConnectionFailed("Failed to get stdin".to_string()))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| TransportError::ConnectionFailed("Failed to get stdout".to_string()))?;

        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let connected = AtomicBool::new(true);

        let transport = Self {
            child: Arc::new(Mutex::new(child)),
            stdin: Arc::new(Mutex::new(stdin)),
            pending: pending.clone(),
            request_id: AtomicU64::new(1),
            connected,
            timeout: std::time::Duration::from_secs(30),
        };

        // Spawn reader task
        let pending_clone = pending;
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                match serde_json::from_str::<Value>(&line) {
                    Ok(response) => {
                        if let Some(id) = response.get("id").and_then(|v| v.as_u64()) {
                            let mut pending_guard = pending_clone.lock().await;
                            if let Some(sender) = pending_guard.remove(&id) {
                                let _ = sender.send(response);
                            }
                        } else {
                            debug!("Received notification: {line}");
                        }
                    }
                    Err(e) => {
                        warn!("Failed to parse response: {e} - {line}");
                    }
                }
            }
        });

        Ok(transport)
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, TransportError> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(TransportError::ConnectionClosed);
        }

        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params.unwrap_or_else(|| json!({}))
        });

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        let message = format!("{}\n", serde_json::to_string(&request)?);
        {
            let mut stdin = self.stdin.lock().await;
            stdin
                .write_all(message.as_bytes())
                .await
                .map_err(|e| TransportError::SendFailed(e.to_string()))?;
            stdin
                .flush()
                .await
                .map_err(|e| TransportError::SendFailed(e.to_string()))?;
        }

        // Wait for response with timeout
        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(response)) => {
                if let Some(error) = response.get("error") {
                    Err(TransportError::ProtocolError(error.to_string()))
                } else {
                    Ok(response.get("result").cloned().unwrap_or(Value::Null))
                }
            }
            Ok(Err(_)) => Err(TransportError::ReceiveFailed("Channel closed".to_string())),
            Err(_) => {
                let mut pending = self.pending.lock().await;
                pending.remove(&id);
                Err(TransportError::Timeout)
            }
        }
    }

    async fn send_notification(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), TransportError> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(TransportError::ConnectionClosed);
        }

        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params.unwrap_or_else(|| json!({}))
        });

        let message = format!("{}\n", serde_json::to_string(&notification)?);
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(message.as_bytes())
            .await
            .map_err(|e| TransportError::SendFailed(e.to_string()))?;
        stdin
            .flush()
            .await
            .map_err(|e| TransportError::SendFailed(e.to_string()))?;

        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.connected.store(false, Ordering::SeqCst);

        let mut child = self.child.lock().await;
        if let Err(e) = child.kill().await {
            error!("Failed to kill subprocess: {e}");
        }

        Ok(())
    }
}

#[cfg(test)]
impl StdioTransport {
    /// Override the request timeout for tests.
    pub fn set_timeout(&mut self, timeout: std::time::Duration) {
        self.timeout = timeout;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]

    use super::*;
    use arkavo_test_macros::spec;
    use std::time::Duration;

    #[spec("MCPR-003")]
    #[tokio::test]
    #[cfg(unix)] // relies on the `sleep` command to simulate a hung server
    async fn send_request_returns_timeout_when_server_hangs() {
        let mut transport = StdioTransport::new(
            "sleep".to_string(),
            vec!["100".to_string()],
            None,
            HashMap::new(),
        )
        .await
        .expect("failed to spawn sleep stub");

        transport.set_timeout(Duration::from_millis(50));

        let result = transport.send_request("tools/call", Some(json!({}))).await;

        assert!(
            matches!(result, Err(TransportError::Timeout)),
            "expected timeout error, got {result:?}"
        );

        transport.close().await.unwrap();
    }
}
