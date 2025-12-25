mod sse;
mod stdio;
mod websocket;

pub use sse::SseTransport;
pub use stdio::StdioTransport;
pub use websocket::WebSocketTransport;

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

/// Create a transport from configuration
#[allow(clippy::implicit_hasher)]
pub async fn create_transport(
    config: TransportConfig,
    env: HashMap<String, String>,
) -> Result<Box<dyn Transport>, TransportError> {
    match config {
        TransportConfig::Stdio { command, args, cwd } => {
            let transport = StdioTransport::new(command, args, cwd, env).await?;
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
