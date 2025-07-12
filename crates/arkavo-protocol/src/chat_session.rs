use crate::error::{A2aError, Result};
use crate::types::{
    ChatCapabilities, ChatSession, MessageDelta, MessageDeltaContent, StreamEndReason, UserMessage,
};
use arkavo_llm::{DeltaType, LlmClientAdapter, StreamLlmModel};
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast, mpsc, oneshot};
use uuid::Uuid;

/// Manages active chat sessions
pub struct ChatSessionManager {
    sessions: Arc<RwLock<HashMap<String, ChatSessionState>>>,
    llm_adapter: Option<Arc<LlmClientAdapter>>,
}

struct ChatSessionState {
    #[allow(dead_code)]
    session: ChatSession,
    message_tx: mpsc::Sender<UserMessage>,
    delta_tx: broadcast::Sender<MessageDelta>,
    _abort_tx: oneshot::Sender<()>,
}

impl ChatSessionManager {
    pub fn new(llm_adapter: Option<Arc<LlmClientAdapter>>) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            llm_adapter,
        }
    }

    /// Create a new chat session
    pub async fn create_session(&self) -> ChatSession {
        let session_id = Uuid::new_v4().to_string();
        let capabilities = ChatCapabilities {
            max_context_length: Some(4096),
            supported_message_types: Some(vec!["text".to_string(), "tool_call".to_string()]),
            supports_attachments: false,
            supports_tools: true,
        };

        let session = ChatSession {
            session_id: session_id.clone(),
            capabilities: Some(capabilities),
            created_at: chrono::Utc::now(),
        };

        // Create channels for this session
        let (message_tx, message_rx) = mpsc::channel::<UserMessage>(32);
        let (delta_tx, _delta_rx) = broadcast::channel::<MessageDelta>(256);
        let (abort_tx, abort_rx) = oneshot::channel();

        // Store session state
        let session_state = ChatSessionState {
            session: session.clone(),
            message_tx,
            delta_tx: delta_tx.clone(),
            _abort_tx: abort_tx,
        };

        self.sessions
            .write()
            .await
            .insert(session_id.clone(), session_state);

        // Start session handler if we have an LLM
        if let Some(llm_adapter) = &self.llm_adapter {
            let session_id_clone = session_id.clone();
            let llm_adapter_clone = llm_adapter.clone();
            let sessions = self.sessions.clone();

            tokio::spawn(async move {
                Self::handle_session(
                    session_id_clone,
                    message_rx,
                    delta_tx,
                    abort_rx,
                    llm_adapter_clone,
                    sessions,
                )
                .await;
            });
        }

        session
    }

    /// Send a message to a session
    pub async fn send_message(&self, session_id: &str, message: UserMessage) -> Result<()> {
        let sessions = self.sessions.read().await;
        if let Some(session_state) = sessions.get(session_id) {
            // Check if we have an LLM adapter to process messages
            if self.llm_adapter.is_none() {
                return Err(A2aError::NoLlmAdapter);
            }

            session_state
                .message_tx
                .send(message)
                .await
                .map_err(|_| A2aError::MessageSendFailed("Channel closed".to_string()))
        } else {
            Err(A2aError::SessionNotFound(session_id.to_string()))
        }
    }

    /// Get a receiver for session deltas
    pub async fn get_delta_stream(&self, session_id: &str) -> Option<mpsc::Receiver<MessageDelta>> {
        let sessions = self.sessions.read().await;
        if let Some(session_state) = sessions.get(session_id) {
            // Subscribe to the broadcast channel
            let mut broadcast_rx = session_state.delta_tx.subscribe();

            // Create an mpsc channel for the subscription
            let (delta_tx, delta_rx) = mpsc::channel(32);

            // Spawn a task to forward from broadcast to mpsc
            tokio::spawn(async move {
                while let Ok(delta) = broadcast_rx.recv().await {
                    if delta_tx.send(delta).await.is_err() {
                        break; // Receiver dropped
                    }
                }
            });

            Some(delta_rx)
        } else {
            None
        }
    }

    /// Close a session
    pub async fn close_session(&self, session_id: &str) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        if sessions.remove(session_id).is_some() {
            // The abort signal will be sent when _abort_tx is dropped
            Ok(())
        } else {
            Err(A2aError::SessionNotFound(session_id.to_string()))
        }
    }

    /// Handle a chat session
    async fn handle_session(
        session_id: String,
        mut message_rx: mpsc::Receiver<UserMessage>,
        delta_tx: broadcast::Sender<MessageDelta>,
        mut abort_rx: oneshot::Receiver<()>,
        llm_adapter: Arc<LlmClientAdapter>,
        sessions: Arc<RwLock<HashMap<String, ChatSessionState>>>,
    ) {
        let mut conversation_context = Vec::new();

        loop {
            tokio::select! {
                // Handle incoming user messages
                Some(user_message) = message_rx.recv() => {
                    // Add to context
                    conversation_context.push(format!("User: {}", user_message.content));

                    // Create chat request with full context
                    let full_context = conversation_context.join("\n");
                    let chat_request = arkavo_llm::ChatRequest::new(full_context);

                    let message_id = Uuid::new_v4().to_string();
                    let trace_id = Uuid::new_v4().to_string();

                    // Start streaming from LLM
                    match llm_adapter.stream_chat(chat_request, trace_id).await {
                        Ok((_stream_id, mut delta_stream)) => {
                            let mut sequence = 0u64;
                            let mut assistant_response = String::new();

                            while let Some(delta_result) = delta_stream.next().await {
                                match delta_result {
                                    Ok(stream_delta) => {
                                        // Convert StreamDelta to MessageDelta
                                        let message_delta = match stream_delta.delta {
                                            DeltaType::Text { content } => {
                                                assistant_response.push_str(&content);
                                                MessageDelta {
                                                    session_id: session_id.clone(),
                                                    message_id: message_id.clone(),
                                                    sequence,
                                                    delta: MessageDeltaContent::Text { text: content },
                                                    timestamp: stream_delta.timestamp,
                                                }
                                            },
                                            DeltaType::ToolCall { id, name, arguments } => MessageDelta {
                                                session_id: session_id.clone(),
                                                message_id: message_id.clone(),
                                                sequence,
                                                delta: MessageDeltaContent::ToolCall {
                                                    tool_call_id: id,
                                                    delta: serde_json::to_string(&serde_json::json!({
                                                        "name": name,
                                                        "arguments": arguments
                                                    })).unwrap_or_default(),
                                                },
                                                timestamp: stream_delta.timestamp,
                                            },
                                            DeltaType::Error(err) => MessageDelta {
                                                session_id: session_id.clone(),
                                                message_id: message_id.clone(),
                                                sequence,
                                                delta: MessageDeltaContent::Error {
                                                    code: err.code,
                                                    message: err.message,
                                                },
                                                timestamp: stream_delta.timestamp,
                                            },
                                            DeltaType::StreamEnd { reason } => {
                                                // Add assistant response to context
                                                if !assistant_response.is_empty() {
                                                    conversation_context.push(format!("Assistant: {assistant_response}"));
                                                }

                                                MessageDelta {
                                                    session_id: session_id.clone(),
                                                    message_id: message_id.clone(),
                                                    sequence,
                                                    delta: MessageDeltaContent::StreamEnd {
                                                        reason: match reason {
                                                            arkavo_llm::EndReason::Complete => StreamEndReason::Complete,
                                                            arkavo_llm::EndReason::MaxTokens => StreamEndReason::MaxTokens,
                                                            arkavo_llm::EndReason::Aborted => StreamEndReason::UserAbort,
                                                            arkavo_llm::EndReason::Error(_) => StreamEndReason::Error,
                                                            arkavo_llm::EndReason::Timeout => StreamEndReason::Error,
                                                        },
                                                    },
                                                    timestamp: stream_delta.timestamp,
                                                }
                                            },
                                        };

                                        sequence += 1;

                                        // Broadcast delta to all subscribers
                                        let _ = delta_tx.send(message_delta);
                                    }
                                    Err(e) => {
                                        eprintln!("Stream error for session {session_id}: {e}");
                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to start LLM stream for session {session_id}: {e}");
                            // Send error delta to subscribers
                            let error_delta = MessageDelta {
                                session_id: session_id.clone(),
                                message_id: Uuid::new_v4().to_string(),
                                sequence: 0,
                                delta: MessageDeltaContent::Error {
                                    code: "LLM_ERROR".to_string(),
                                    message: format!("Failed to start LLM stream: {e}"),
                                },
                                timestamp: chrono::Utc::now(),
                            };
                            let _ = delta_tx.send(error_delta);
                        }
                    }
                }

                // Handle abort signal
                _ = &mut abort_rx => {
                    // Send stream end delta
                    let end_delta = MessageDelta {
                        session_id: session_id.clone(),
                        message_id: Uuid::new_v4().to_string(),
                        sequence: 0,
                        delta: MessageDeltaContent::StreamEnd {
                            reason: StreamEndReason::SessionClosed,
                        },
                        timestamp: chrono::Utc::now(),
                    };
                    let _ = delta_tx.send(end_delta);
                    break;
                }
            }
        }

        // Clean up session
        sessions.write().await.remove(&session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_session_creation() {
        let manager = ChatSessionManager::new(None);
        let session = manager.create_session().await;

        assert!(!session.session_id.is_empty());
        assert!(session.capabilities.is_some());
    }

    #[tokio::test]
    async fn test_session_lifecycle() {
        let manager = ChatSessionManager::new(None);
        let session = manager.create_session().await;
        let session_id = session.session_id.clone();

        // Send a message without LLM adapter should fail
        let message = UserMessage {
            content: "Hello".to_string(),
            attachments: None,
            metadata: None,
        };

        let result = manager.send_message(&session_id, message).await;
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(matches!(e, crate::error::A2aError::NoLlmAdapter));
        }

        // Close session
        assert!(manager.close_session(&session_id).await.is_ok());

        // Try to send to closed session
        let message2 = UserMessage {
            content: "Hello again".to_string(),
            attachments: None,
            metadata: None,
        };
        let result2 = manager.send_message(&session_id, message2).await;
        assert!(result2.is_err());
        if let Err(e) = result2 {
            assert!(matches!(e, crate::error::A2aError::SessionNotFound(_)));
        }
    }
}
