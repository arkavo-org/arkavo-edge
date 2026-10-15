mod sse;
mod stdio;
mod websocket;

pub use sse::SseTransport;
pub use stdio::StdioTransport;
pub use websocket::WebSocketTransport;

use arkavo_process_env::{ChildEnv, EnvSpec};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

/// Configuration for different transport types
#[derive(Debug, Clone)]
pub enum TransportConfig {
    /// Stdio transport spawning a subprocess
    Stdio {
        command: String,
        args: Vec<String>,
        cwd: Option<String>,
    },
    /// HTTP Server-Sent Events transport
    Sse {
        url: String,
        headers: HashMap<String, String>,
    },
    /// WebSocket transport
    WebSocket {
        url: String,
        headers: HashMap<String, String>,
    },
}

/// Transport trait for MCP client-server communication
#[async_trait]
pub trait Transport: Send + Sync {
    /// Send a JSON-RPC request and wait for response
    async fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, TransportError>;

    /// Send a JSON-RPC notification (no response expected)
    async fn send_notification(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), TransportError>;

    /// Check if the transport is connected
    fn is_connected(&self) -> bool;

    /// Close the transport connection
    async fn close(&self) -> Result<(), TransportError>;
}

/// Transport error types
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Send failed: {0}")]
    SendFailed(String),

    #[error("Receive failed: {0}")]
    ReceiveFailed(String),

    #[error("Timeout")]
    Timeout,

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Protocol error: {0}")]
    ProtocolError(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Create a transport from configuration. A Stdio server's environment is
/// resolved from `env` and this process's environment; see
/// [`ChildEnv::isolated`].
pub async fn create_transport(
    config: TransportConfig,
    env: &EnvSpec,
) -> Result<Box<dyn Transport>, TransportError> {
    match config {
        TransportConfig::Stdio { command, args, cwd } => {
            let env = ChildEnv::isolated_from_current(env);
            let transport = StdioTransport::new(command, args, cwd, &env).await?;
            Ok(Box::new(transport))
        }
        TransportConfig::Sse { url, headers } => {
            let transport = SseTransport::new(url, headers).await?;
            Ok(Box::new(transport))
        }
        TransportConfig::WebSocket { url, headers } => {
            let transport = WebSocketTransport::new(url, headers).await?;
            Ok(Box::new(transport))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]

    use super::*;
    use arkavo_test_macros::spec;

    /// create_transport is the runtime client's spawn path: the server must
    /// start with the resolved environment only, not this process's.
    #[spec("MCPR-008")]
    #[tokio::test]
    #[cfg(unix)]
    async fn create_transport_stdio_server_sees_only_the_resolved_environment() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dump = dir.path().join("env");
        let spec = EnvSpec {
            set: [("ENV_DUMP_OUT".to_string(), dump.display().to_string())].into(),
            passthrough: Vec::new(),
        };

        let transport = create_transport(
            TransportConfig::Stdio {
                command: "sh".to_string(),
                args: vec!["-c".to_string(), r#"env > "$ENV_DUMP_OUT""#.to_string()],
                cwd: None,
            },
            &spec,
        )
        .await
        .expect("spawn environment dump");

        for _ in 0..50 {
            if dump.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let seen = std::fs::read_to_string(&dump).expect("environment dump written");
        assert!(
            seen.lines().any(|l| l.starts_with("ENV_DUMP_OUT=")),
            "{seen}"
        );
        assert!(
            !seen.lines().any(|l| l.starts_with("CARGO_MANIFEST_DIR=")),
            "this process's own environment leaked into the MCP server: {seen}"
        );

        transport.close().await.expect("close");
    }
}
