//! One agent's RPC endpoint, as the mesh tools and [`crate::send_and_wait`]
//! talk to it.
//!
//! Sending a message and reading a task back are the same two calls whoever
//! makes them. Keeping them here means a delegation and a request that waits
//! for its answer cannot drift apart in how they reach an agent or read its
//! reply.

// The module is private, so nothing here leaves the crate. `pub(crate)` on
// these items trips clippy's `redundant_pub_crate` and plain `pub` trips
// rustc's `unreachable_pub`; the repo settles it this way elsewhere too.
#![allow(unreachable_pub)]

use std::sync::Arc;

use arkavo_protocol::transport::TlsConfig;
use arkavo_protocol::types::{
    Message, MessagePart, MessageSendRequest, MessageSendResponse, TaskGetRequest, TaskGetResponse,
};
use arkavo_protocol::{
    A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, HttpTransport, TransportConfig,
};
use serde_json::json;

/// Why a call to an agent produced no usable reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerError {
    /// The HTTP client could not be built.
    Transport(String),
    /// The endpoint was refused before anything was sent.
    Connect(String),
    /// The request was sent and no reply came back.
    Request(String),
    /// The reply was not the shape the method returns.
    Parse(String),
    /// The agent answered with a JSON-RPC error.
    Rpc { code: i32, message: String },
}

impl PeerError {
    /// True when the agent itself answered, as opposed to the call failing
    /// on the way to it.
    pub const fn is_answer(&self) -> bool {
        matches!(self, Self::Rpc { .. })
    }
}

impl std::fmt::Display for PeerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(e) => write!(f, "Failed to create transport: {e}"),
            Self::Connect(e) => write!(f, "Failed to connect to agent: {e}"),
            Self::Request(e) => write!(f, "Request failed: {e}"),
            Self::Parse(e) => write!(f, "Failed to parse response: {e}"),
            Self::Rpc { code, message } => write!(f, "{code}: {message}"),
        }
    }
}

/// How patient a call is. Each caller keeps the limits it had before the
/// calls were shared.
#[derive(Debug, Clone, Copy)]
pub struct Patience {
    pub timeout_ms: u64,
    pub max_retries: u32,
}

/// A connection to one agent.
pub struct Peer {
    transport: Arc<HttpTransport>,
}

impl Peer {
    pub async fn connect(
        address: &str,
        agent_id: &str,
        patience: Patience,
    ) -> Result<Self, PeerError> {
        let config = TransportConfig {
            timeout_ms: patience.timeout_ms,
            max_retries: patience.max_retries,
            tls_config: TlsConfig {
                require_tls: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let transport =
            Arc::new(HttpTransport::new(config).map_err(|e| PeerError::Transport(e.to_string()))?);
        let endpoint = A2aEndpoint {
            url: address.to_string(),
            agent_id: agent_id.to_string(),
            public_key: None,
        };
        transport
            .connect(&endpoint)
            .await
            .map_err(|e| PeerError::Connect(e.to_string()))?;
        Ok(Self { transport })
    }

    /// `message/send`: hand the agent a message and get the task it opened.
    pub async fn send_message(&self, message: Message) -> Result<MessageSendResponse, PeerError> {
        let request = MessageSendRequest {
            message,
            task_id: None,
        };
        self.call("message/send", json!([request])).await
    }

    /// `tasks/get`: read a task's state, and its result once it has one.
    pub async fn get_task(&self, task_id: &str) -> Result<TaskGetResponse, PeerError> {
        let request = TaskGetRequest {
            task_id: task_id.to_string(),
        };
        self.call("tasks/get", json!([request])).await
    }

    pub async fn close(self) {
        let _ = self.transport.close().await;
    }

    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<T, PeerError> {
        let response = self
            .transport
            .send_request(A2aRequest::new(method, params))
            .await
            .map_err(|e| PeerError::Request(e.to_string()))?;
        match response {
            A2aResponse::Success { result, .. } => {
                serde_json::from_value(result).map_err(|e| PeerError::Parse(e.to_string()))
            }
            A2aResponse::Error { error, .. } => Err(PeerError::Rpc {
                code: error.code,
                message: error.message,
            }),
        }
    }
}

/// The text of a message: its text parts, in order, one per line.
pub fn text_of(message: &Message) -> String {
    message
        .parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text { content } => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
