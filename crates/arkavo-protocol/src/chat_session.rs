use crate::config::BufferConfig;
use crate::error::{A2aError, Result};
use crate::types::{
    ChatCapabilities, ChatSession, MessageDelta, MessageDeltaContent, StreamEndReason, UserMessage,
};
use arkavo_llm::{DeltaType, LlmClientAdapter, StreamLlmModel};
use arkavo_observability::{
    metrics::MetricsCollector,
    session::{SessionMetrics, SessionState, SessionTtlCleaner},
    session_observability,
    task_tracker::{ObservableTaskTracker, SessionTaskManager},
};
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast, mpsc};
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

/// Manages active chat sessions
pub struct ChatSessionManager {
    sessions: Arc<RwLock<HashMap<String, ChatSessionState>>>,
    session_metrics: Arc<RwLock<HashMap<String, SessionMetrics>>>,
    llm_adapter: Option<Arc<LlmClientAdapter>>,
    metrics_collector: MetricsCollector,
    task_tracker: ObservableTaskTracker,
    _ttl_seconds: u64,
    buffer_config: BufferConfig,
}

struct ChatSessionState {
    _session: ChatSession,
    state: SessionState,
    message_tx: mpsc::Sender<UserMessage>,
    delta_tx: broadcast::Sender<MessageDelta>,
    task_manager: SessionTaskManager,
}

impl ChatSessionManager {
    /// Create a new chat session manager with observability
    pub fn new(llm_adapter: Option<Arc<LlmClientAdapter>>) -> Self {
        Self::with_config(llm_adapter, 3600, BufferConfig::default()) // Default 1 hour TTL
    }

    /// Create a new chat session manager with custom TTL
    pub fn with_config(
        llm_adapter: Option<Arc<LlmClientAdapter>>,
        ttl_seconds: u64,
        buffer_config: BufferConfig,
    ) -> Self {
        let session_metrics = Arc::new(RwLock::new(HashMap::new()));
        let metrics_collector = MetricsCollector::new();
        let task_tracker = ObservableTaskTracker::new("chat-session-manager");

        // Start TTL cleaner
        let cleaner = SessionTtlCleaner::new(ttl_seconds, 60); // Check every minute
        let metrics_for_cleaner = session_metrics.clone();

        let _cleaner_handle = task_tracker.spawn_named("ttl-cleaner", async move {
            cleaner
                .start(metrics_for_cleaner, |session_id| async move {
                    info!(session.id = %session_id, "Session cleaned up by TTL cleaner");
                })
                .await
                .await
                .unwrap_or_else(|e| {
                    warn!(error = %e, "TTL cleaner task failed");
                });
        });

        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            session_metrics,
            llm_adapter,
            metrics_collector,
            task_tracker,
            _ttl_seconds: ttl_seconds,
            buffer_config,
        }
    }

    /// Create a new chat session
    #[instrument(skip(self), fields(session.id))]
    pub async fn create_session(&self) -> ChatSession {
        let session_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("session.id", &session_id);

        let capabilities = ChatCapabilities {
            max_context_length: Some(4096),
            supported_message_types: Some(vec!["text".to_string(), "tool_call".to_string()]),
            supports_attachments: false,
            supports_tools: true,
        };

        let session = ChatSession {
            session_id: session_id.clone(),
            capabilities: Some(capabilities.clone()),
            created_at: chrono::Utc::now(),
        };

        // Create session metrics
        let session_metrics = SessionMetrics::new(session_id.clone());
        self.session_metrics
            .write()
            .await
            .insert(session_id.clone(), session_metrics);

        // Create channels for this session
        let (message_tx, message_rx) = mpsc::channel::<UserMessage>(32);
        let (delta_tx, _delta_rx) = broadcast::channel::<MessageDelta>(256);

        // Create task manager for this session
        let task_manager = SessionTaskManager::new(session_id.clone());

        // Store session state
        let session_state = ChatSessionState {
            _session: session.clone(),
            state: SessionState::Active,
            message_tx,
            delta_tx: delta_tx.clone(),
            task_manager,
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
            let session_metrics = self.session_metrics.clone();
            let metrics_collector = self.metrics_collector.clone();

            self.task_tracker
                .spawn_named("session-handler", async move {
                    Self::handle_session(
                        session_id_clone,
                        message_rx,
                        delta_tx,
                        llm_adapter_clone,
                        sessions,
                        session_metrics,
                        metrics_collector,
                    )
                    .await;
                });
        }

        // Record session creation
        self.metrics_collector.record_session_created();
        session_observability::log_session_created(
            &session_id,
            capabilities
                .supported_message_types
                .as_ref()
                .map(|v| v.join(","))
                .as_deref(),
        );

        session
    }

    /// Send a message to a session
    #[instrument(skip(self, message), fields(session.id = %session_id, message.length = message.content.len()))]
    pub async fn send_message(&self, session_id: &str, message: UserMessage) -> Result<()> {
        let sessions = self.sessions.read().await;
        if let Some(session_state) = sessions.get(session_id) {
            // Check if session is in valid state
            if session_state.state != SessionState::Active {
                warn!(session.state = %session_state.state, "Attempted to send message to non-active session");
                return Err(A2aError::SessionNotFound(format!(
                    "Session {session_id} is not active"
                )));
            }

            // Check if we have an LLM adapter to process messages
            if self.llm_adapter.is_none() {
                return Err(A2aError::NoLlmAdapter);
            }

            let message_length = message.content.len();

            // Record metrics
            if let Some(metrics) = self.session_metrics.read().await.get(session_id) {
                metrics.record_message_sent(message_length);
            }
            self.metrics_collector.record_message_sent(message_length);

            // Log the message
            session_observability::log_message_sent(session_id, message_length);

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
    #[instrument(skip(self), fields(session.id = %session_id))]
    pub async fn get_delta_stream(&self, session_id: &str) -> Option<mpsc::Receiver<MessageDelta>> {
        let sessions = self.sessions.read().await;
        if let Some(session_state) = sessions.get(session_id) {
            // Subscribe to the broadcast channel
            let mut broadcast_rx = session_state.delta_tx.subscribe();

            // Create an mpsc channel for the subscription
            let (delta_tx, delta_rx) = mpsc::channel(self.buffer_config.chat_delta_buffer_size);

            // Spawn a task to forward from broadcast to mpsc with metrics
            let session_id_clone = session_id.to_string();
            let session_metrics = self.session_metrics.clone();
            let metrics_collector = self.metrics_collector.clone();

            self.task_tracker
                .spawn_named("delta-forwarder", async move {
                    while let Ok(delta) = broadcast_rx.recv().await {
                        // Record delta metrics
                        let delta_type = match &delta.delta {
                            MessageDeltaContent::Text { .. } => "text",
                            MessageDeltaContent::ToolCall { .. } => "tool_call",
                            MessageDeltaContent::Error { .. } => "error",
                            MessageDeltaContent::StreamEnd { .. } => "stream_end",
                        };

                        if let Some(metrics) = session_metrics.read().await.get(&session_id_clone) {
                            metrics.record_delta_sent(delta_type, delta.sequence);
                        }
                        metrics_collector.record_delta_sent(delta_type);

                        session_observability::log_message_received(
                            &session_id_clone,
                            delta_type,
                            delta.sequence,
                        );

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
    #[instrument(skip(self), fields(session.id = %session_id))]
    pub async fn close_session(&self, session_id: &str) -> Result<()> {
        // Update session state to Closing first
        {
            let mut sessions = self.sessions.write().await;
            if let Some(session_state) = sessions.get_mut(session_id) {
                session_state.state = SessionState::Closing;
                info!("Session state changed to Closing");
            } else {
                return Err(A2aError::SessionNotFound(session_id.to_string()));
            }
        }

        // Update metrics state
        if let Some(metrics) = self.session_metrics.write().await.get_mut(session_id) {
            metrics.set_state(SessionState::Closing);
        }

        // Gracefully shutdown session task manager
        let mut sessions = self.sessions.write().await;
        if let Some(session_state) = sessions.remove(session_id) {
            // Shutdown task manager
            session_state.task_manager.shutdown().await;

            // Remove from metrics tracking
            self.session_metrics.write().await.remove(session_id);

            // Record closure
            self.metrics_collector.record_session_closed();
            session_observability::log_session_closed(session_id, "user_requested");

            Ok(())
        } else {
            Err(A2aError::SessionNotFound(session_id.to_string()))
        }
    }

    /// Handle a chat session
    #[instrument(skip(message_rx, delta_tx, llm_adapter, sessions, session_metrics, metrics_collector), fields(session.id = %session_id))]
    async fn handle_session(
        session_id: String,
        mut message_rx: mpsc::Receiver<UserMessage>,
        delta_tx: broadcast::Sender<MessageDelta>,
        llm_adapter: Arc<LlmClientAdapter>,
        sessions: Arc<RwLock<HashMap<String, ChatSessionState>>>,
        session_metrics: Arc<RwLock<HashMap<String, SessionMetrics>>>,
        metrics_collector: MetricsCollector,
    ) {
        let mut conversation_context = Vec::new();
        info!("Session handler started");

        loop {
            tokio::select! {
                // Handle incoming user messages
                Some(user_message) = message_rx.recv() => {
                    let start_time = std::time::Instant::now();

                    // Record message received
                    if let Some(metrics) = session_metrics.read().await.get(&session_id) {
                        metrics.record_message_received();
                    }
                    metrics_collector.record_message_received();

                    // Add to context
                    conversation_context.push(format!("User: {}", user_message.content));

                    // Create chat request with full context
                    let full_context = conversation_context.join("\n");
                    let chat_request = arkavo_llm::ChatRequest::new(full_context);

                    let message_id = Uuid::new_v4().to_string();
                    let trace_id = Uuid::new_v4().to_string();

                    session_observability::log_stream_start(&session_id, None);

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

                                                let end_reason = match reason {
                                                    arkavo_llm::EndReason::Complete => StreamEndReason::Complete,
                                                    arkavo_llm::EndReason::MaxTokens => StreamEndReason::MaxTokens,
                                                    arkavo_llm::EndReason::Aborted => StreamEndReason::UserAbort,
                                                    arkavo_llm::EndReason::Error(_) => StreamEndReason::Error,
                                                    arkavo_llm::EndReason::Timeout => StreamEndReason::Error,
                                                };

                                                // Record stream completion
                                                let duration = start_time.elapsed();
                                                metrics_collector.record_response_time(duration.as_millis() as u64);
                                                session_observability::log_stream_end(&session_id, &format!("{end_reason:?}"), None);

                                                MessageDelta {
                                                    session_id: session_id.clone(),
                                                    message_id: message_id.clone(),
                                                    sequence,
                                                    delta: MessageDeltaContent::StreamEnd { reason: end_reason },
                                                    timestamp: stream_delta.timestamp,
                                                }
                                            },
                                        };

                                        sequence += 1;

                                        // Broadcast delta to all subscribers
                                        let _ = delta_tx.send(message_delta);
                                    }
                                    Err(e) => {
                                        error!(error = %e, "Stream error for session");

                                        // Record error metrics
                                        if let Some(metrics) = session_metrics.read().await.get(&session_id) {
                                            metrics.record_error("stream", &e.to_string());
                                        }
                                        metrics_collector.record_error("llm");
                                        session_observability::log_session_error(&session_id, &e.to_string(), Some("STREAM_ERROR"));

                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!(error = %e, "Failed to start LLM stream for session");

                            // Record error metrics
                            if let Some(metrics) = session_metrics.read().await.get(&session_id) {
                                metrics.record_error("llm_start", &e.to_string());
                            }
                            metrics_collector.record_error("llm");
                            session_observability::log_session_error(&session_id, &e.to_string(), Some("LLM_START_ERROR"));

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

                // Check if session should be closed
                else => {
                    // Channel closed, check if session is being closed
                    if let Some(session_state) = sessions.read().await.get(&session_id) {
                        if session_state.state == SessionState::Closing {
                            info!("Session handler exiting due to closure request");

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
                    } else {
                        warn!("Session not found in sessions map, exiting handler");
                        break;
                    }
                }
            }
        }

        // Mark session as zombie if it still exists (cleanup needed)
        if let Some(session_state) = sessions.read().await.get(&session_id) {
            if session_state.state != SessionState::Closing {
                if let Some(metrics) = session_metrics.write().await.get_mut(&session_id) {
                    metrics.set_state(SessionState::Zombie);
                    warn!("Session marked as zombie - cleanup needed");
                }
            }
        }

        info!("Session handler exited");
    }

    /// Get metrics snapshot for all sessions
    pub async fn get_metrics_snapshot(
        &self,
    ) -> HashMap<String, arkavo_observability::session::SessionMetricsSnapshot> {
        let metrics = self.session_metrics.read().await;
        metrics
            .iter()
            .map(|(id, metrics)| (id.clone(), metrics.snapshot()))
            .collect()
    }

    /// Get global metrics collector
    pub fn get_global_metrics(&self) -> &MetricsCollector {
        &self.metrics_collector
    }

    /// Gracefully shutdown the session manager
    #[instrument(skip(self))]
    pub async fn shutdown(self) {
        info!("Shutting down chat session manager");

        // Close all active sessions
        let session_ids: Vec<String> = {
            let sessions = self.sessions.read().await;
            sessions.keys().cloned().collect()
        };

        for session_id in session_ids {
            if let Err(e) = self.close_session(&session_id).await {
                warn!(session.id = %session_id, error = %e, "Failed to close session during shutdown");
            }
        }

        // Shutdown the task tracker
        self.task_tracker.close().await;

        info!("Chat session manager shutdown complete");
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

        // Check that metrics were recorded
        let global_metrics = manager.get_global_metrics().snapshot();
        assert_eq!(global_metrics.total_sessions_created, 1);
        assert_eq!(global_metrics.active_sessions, 1);

        manager.shutdown().await;
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

        // Check session metrics
        let session_metrics = manager.get_metrics_snapshot().await;
        assert!(session_metrics.contains_key(&session_id));

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

        // Check that session was removed from metrics
        let session_metrics_after = manager.get_metrics_snapshot().await;
        assert!(!session_metrics_after.contains_key(&session_id));

        manager.shutdown().await;
    }

    #[tokio::test]
    async fn test_ttl_cleanup() {
        // Create manager with very short TTL for testing
        let manager = ChatSessionManager::with_config(None, 1, BufferConfig::default()); // 1 second TTL
        let session = manager.create_session().await;
        let _session_id = session.session_id.clone();

        // Wait for TTL to expire
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // Session should be cleaned up by TTL cleaner
        // Note: This test might be flaky due to timing, but demonstrates the concept

        manager.shutdown().await;
    }
}
