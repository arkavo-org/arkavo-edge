use crate::a2a_mcp_bridge::{A2aMcpBridge, McpToolRequest, McpToolResponse};
use crate::chat_session::ChatSessionManager;
use crate::types::{MessageDelta, UserMessage};
use arkavo_mcp_tools::ToolRegistry;
use arkavo_router::Router;
use serde_json::Value;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc;

/// A2A client for agent-to-agent communication with chat session support
pub struct A2aClient {
    mcp_bridge: Option<A2aMcpBridge>,
    session_manager: Option<ChatSessionManager>,
    session_id: Option<String>,
}

impl Default for A2aClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors specific to A2A client chat operations
#[derive(Debug, Error)]
pub enum A2aClientError {
    #[error("Session manager not initialized - use with_router()")]
    NoSessionManager,
    #[error("No active session - call open_session() first")]
    NoSession,
    #[error("Failed to send message: {0}")]
    SendFailed(String),
    #[error("Failed to get delta stream")]
    StreamFailed,
    #[error("Failed to close session: {0}")]
    CloseFailed(String),
}

impl A2aClient {
    pub fn new() -> Self {
        Self {
            mcp_bridge: None,
            session_manager: None,
            session_id: None,
        }
    }

    pub fn with_mcp_bridge(mcp_bridge: A2aMcpBridge) -> Self {
        Self {
            mcp_bridge: Some(mcp_bridge),
            session_manager: None,
            session_id: None,
        }
    }

    /// Create A2A client with router for chat sessions
    pub fn with_router(router: Arc<Router>, tool_registry: Option<Arc<ToolRegistry>>) -> Self {
        let session_manager = ChatSessionManager::with_router(router, tool_registry);
        Self {
            mcp_bridge: None,
            session_manager: Some(session_manager),
            session_id: None,
        }
    }

    /// Open a chat session
    pub async fn open_session(&mut self) -> Result<String, A2aClientError> {
        let manager = self
            .session_manager
            .as_ref()
            .ok_or(A2aClientError::NoSessionManager)?;

        let session = manager.create_session(None).await;
        let session_id = session.session_id;
        self.session_id = Some(session_id.clone());
        Ok(session_id)
    }

    /// Send message and get streaming response
    pub async fn send_message(
        &self,
        content: &str,
    ) -> Result<mpsc::Receiver<MessageDelta>, A2aClientError> {
        let manager = self
            .session_manager
            .as_ref()
            .ok_or(A2aClientError::NoSessionManager)?;
        let session_id = self.session_id.as_ref().ok_or(A2aClientError::NoSession)?;

        let message = UserMessage {
            content: content.to_string(),
            attachments: None,
            metadata: None,
        };

        manager
            .send_message(session_id, message)
            .await
            .map_err(|e| A2aClientError::SendFailed(e.to_string()))?;

        manager
            .get_delta_stream(session_id)
            .await
            .ok_or(A2aClientError::StreamFailed)
    }

    /// Close current session
    pub async fn close_session(&mut self) -> Result<(), A2aClientError> {
        if let (Some(manager), Some(session_id)) = (&self.session_manager, self.session_id.take()) {
            manager
                .close_session(&session_id)
                .await
                .map_err(|e| A2aClientError::CloseFailed(e.to_string()))?;
        }
        Ok(())
    }

    /// Get current session ID
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Check if session is active
    pub fn has_session(&self) -> bool {
        self.session_id.is_some()
    }

    // Existing methods unchanged
    pub fn send(&self, _message: &str) -> Result<String, Box<dyn std::error::Error>> {
        Ok("A2A response".to_string())
    }

    pub async fn call_mcp_tool(
        &self,
        tool_name: &str,
        params: Value,
    ) -> Result<McpToolResponse, Box<dyn std::error::Error>> {
        let bridge = self
            .mcp_bridge
            .as_ref()
            .ok_or("MCP bridge not initialized")?;

        let request = McpToolRequest {
            tool_name: tool_name.to_string(),
            params,
        };

        Ok(bridge.call_tool(request).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a2a_client_default() {
        let client = A2aClient::default();
        assert!(!client.has_session());
        assert!(client.session_id().is_none());
    }

    #[test]
    fn test_a2a_client_error_display() {
        let err = A2aClientError::NoSessionManager;
        assert_eq!(
            err.to_string(),
            "Session manager not initialized - use with_router()"
        );

        let err = A2aClientError::NoSession;
        assert_eq!(
            err.to_string(),
            "No active session - call open_session() first"
        );

        let err = A2aClientError::SendFailed("channel closed".to_string());
        assert_eq!(err.to_string(), "Failed to send message: channel closed");
    }
}
