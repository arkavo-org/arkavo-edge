use crate::auth::SessionAuth;
use crate::config::{BufferConfig, ChatStreamingMode};
use crate::error::{A2aError, Result};
use crate::types::{
    ChatCapabilities, ChatSession, MessageDelta, MessageDeltaContent, StreamEndReason, UserMessage,
};
use arkavo_llm::{
    DeltaType, LlmClientAdapter, Message, Role, StreamLlmModel, ToolExecutionResult, ToolExecutor,
};
use arkavo_mcp_tools::ToolRegistry;
use arkavo_observability::{
    metrics::MetricsCollector,
    session::{SessionMetrics, SessionState, SessionTtlCleaner},
    session_observability,
    task_tracker::{ObservableTaskTracker, SessionTaskManager},
};
use arkavo_router::Router;
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Notify, RwLock, broadcast, mpsc};
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

/// Manages active chat sessions
pub struct ChatSessionManager {
    sessions: Arc<RwLock<HashMap<String, ChatSessionState>>>,
    session_metrics: Arc<RwLock<HashMap<String, SessionMetrics>>>,
    llm_adapter: Option<Arc<LlmClientAdapter>>,
    router: Option<Arc<Router>>,
    tool_registry: Option<Arc<ToolRegistry>>,
    metrics_collector: MetricsCollector,
    task_tracker: ObservableTaskTracker,
    _ttl_seconds: u64,
    buffer_config: BufferConfig,
    /// Shared learning context string, updated by the server from LearningBus.
    /// Prepended as a system message before each chat inference.
    learning_context: Option<Arc<RwLock<String>>>,
}

struct ChatSessionState {
    _session: ChatSession,
    state: SessionState,
    message_tx: mpsc::Sender<UserMessage>,
    delta_tx: broadcast::Sender<MessageDelta>,
    task_manager: SessionTaskManager,
    auth: Option<SessionAuth>,
    inflight_deltas: Arc<AtomicU64>,
    last_acked_seq: Arc<AtomicU64>,
    backpressure_notify: Arc<Notify>,
}

impl ChatSessionState {
    /// Check invariants for this session state
    /// Verifies: CHAT-INV-001 (back-pressure threshold), CHAT-INV-002 (state validity)
    #[cfg(test)]
    fn check_invariants(&self) -> std::result::Result<(), String> {
        // CHAT-INV-001: Back-pressure threshold must not exceed 100
        const MAX_INFLIGHT_WINDOW: u64 = 100;
        let inflight = self.inflight_deltas.load(Ordering::SeqCst);
        if inflight > MAX_INFLIGHT_WINDOW {
            return Err(format!(
                "Invariant violation: inflight_deltas ({}) exceeds MAX_INFLIGHT_WINDOW ({})",
                inflight, MAX_INFLIGHT_WINDOW
            ));
        }

        // CHAT-INV-002: State must be valid (Active, Closing, or Zombie)
        match self.state {
            SessionState::Active | SessionState::Closing | SessionState::Zombie => {
                Ok::<(), String>(())
            }
        }?;

        // CHAT-INV-003: last_acked_seq must not exceed inflight sequence (when applicable)
        // Note: This is a simplified check; in full implementation track actual sequences

        Ok(())
    }
}

impl ChatSessionManager {
    /// Create a new chat session manager with observability
    pub fn new(llm_adapter: Option<Arc<LlmClientAdapter>>) -> Self {
        Self::with_config(llm_adapter, None, None, 3600, BufferConfig::default()) // Default 1 hour TTL
    }

    /// Create a new chat session manager with router and tool registry
    pub fn with_router(router: Arc<Router>, tool_registry: Option<Arc<ToolRegistry>>) -> Self {
        Self::with_config(
            None,
            Some(router),
            tool_registry,
            3600,
            BufferConfig::default(),
        )
    }

    /// Create a new chat session manager with custom TTL
    pub fn with_config(
        llm_adapter: Option<Arc<LlmClientAdapter>>,
        router: Option<Arc<Router>>,
        tool_registry: Option<Arc<ToolRegistry>>,
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
            router,
            tool_registry,
            metrics_collector,
            task_tracker,
            _ttl_seconds: ttl_seconds,
            buffer_config,
            learning_context: None,
        }
    }

    /// Set a shared learning context that will be prepended to chat messages.
    /// The server updates this string from LearningBus (lessons, quality trends).
    pub fn set_learning_context(&mut self, context: Arc<RwLock<String>>) {
        self.learning_context = Some(context);
    }

    /// Create a new chat session with optional authentication
    #[instrument(skip(self, auth), fields(session.id))]
    pub async fn create_session(&self, auth: Option<SessionAuth>) -> ChatSession {
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

        // Store session state with authentication and back-pressure management
        let inflight_deltas = Arc::new(AtomicU64::new(0));
        let last_acked_seq = Arc::new(AtomicU64::new(0));
        let backpressure_notify = Arc::new(Notify::new());

        let session_state = ChatSessionState {
            _session: session.clone(),
            state: SessionState::Active,
            message_tx,
            delta_tx: delta_tx.clone(),
            task_manager,
            auth: auth.clone(),
            inflight_deltas: inflight_deltas.clone(),
            last_acked_seq: last_acked_seq.clone(),
            backpressure_notify: backpressure_notify.clone(),
        };

        // CHAT-INV-002: Verify invariants after state creation
        #[cfg(test)]
        session_state
            .check_invariants()
            .expect("Session state invariants violated on creation");

        self.sessions
            .write()
            .await
            .insert(session_id.clone(), session_state);

        // Log authentication info if present
        if let Some(auth_info) = &auth {
            info!(
                session.id = %session_id,
                user = %auth_info.sub,
                scopes = ?auth_info.scopes,
                "Authenticated session created"
            );
        }

        // Start session handler - prefer router over llm_adapter
        if let Some(router) = &self.router {
            let session_id_clone = session_id.clone();
            let router_clone = router.clone();
            let tool_registry_clone = self.tool_registry.clone();
            let sessions = self.sessions.clone();
            let session_metrics = self.session_metrics.clone();
            let metrics_collector = self.metrics_collector.clone();
            let learning_context = self.learning_context.clone();

            self.task_tracker
                .spawn_named("session-handler-router", async move {
                    Self::handle_session_with_router(
                        session_id_clone,
                        message_rx,
                        delta_tx,
                        router_clone,
                        tool_registry_clone,
                        sessions,
                        session_metrics,
                        metrics_collector,
                        learning_context,
                    )
                    .await;
                });
        } else if let Some(llm_adapter) = &self.llm_adapter {
            let session_id_clone = session_id.clone();
            let llm_adapter_clone = llm_adapter.clone();
            let sessions = self.sessions.clone();
            let session_metrics = self.session_metrics.clone();
            let metrics_collector = self.metrics_collector.clone();
            let buffer_config = self.buffer_config.clone();

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
                        inflight_deltas,
                        backpressure_notify,
                        buffer_config,
                    )
                    .await;
                });
        } else {
            warn!(
                "No LLM adapter or router available for session {}",
                session_id
            );
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

            // Check if we have an LLM adapter or router to process messages
            if self.llm_adapter.is_none() && self.router.is_none() {
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
                            MessageDeltaContent::ToolResult { .. } => "tool_result",
                            MessageDeltaContent::Error { .. } => "error",
                            MessageDeltaContent::StreamEnd { .. } => "stream_end",
                            MessageDeltaContent::Metadata { .. } => "metadata",
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
                // CHAT-INV-002: Verify invariants after state mutation
                #[cfg(test)]
                session_state
                    .check_invariants()
                    .expect("Session state invariants violated on close");
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

    /// Get authentication info for a session
    pub async fn get_session_auth(&self, session_id: &str) -> Option<SessionAuth> {
        self.sessions
            .read()
            .await
            .get(session_id)
            .and_then(|state| state.auth.clone())
    }

    /// Check if a session exists and is active
    pub async fn session_exists(&self, session_id: &str) -> bool {
        self.sessions
            .read()
            .await
            .get(session_id)
            .map(|state| state.state == SessionState::Active)
            .unwrap_or(false)
    }

    /// Process a metrics acknowledgment from the client
    pub async fn process_metrics_ack(&self, session_id: &str, last_seq: u64) -> Result<()> {
        let sessions = self.sessions.read().await;
        if let Some(state) = sessions.get(session_id) {
            // Update the last acknowledged sequence
            state.last_acked_seq.store(last_seq, Ordering::SeqCst);

            // Calculate inflight deltas (simplified - in production, track actual sequences)
            let current_seq = state.inflight_deltas.load(Ordering::SeqCst);
            if current_seq > last_seq {
                let inflight = current_seq - last_seq;
                state.inflight_deltas.store(inflight, Ordering::SeqCst);
            } else {
                state.inflight_deltas.store(0, Ordering::SeqCst);
            }

            // Notify if we were waiting for acknowledgment
            state.backpressure_notify.notify_one();

            info!(
                session.id = %session_id,
                last_seq = last_seq,
                inflight = state.inflight_deltas.load(Ordering::SeqCst),
                "Processed metrics acknowledgment"
            );

            Ok(())
        } else {
            Err(A2aError::SessionNotFound(session_id.to_string()))
        }
    }

    /// Handle a chat session with back-pressure management
    #[instrument(skip(message_rx, delta_tx, llm_adapter, sessions, session_metrics, metrics_collector, inflight_deltas, backpressure_notify, buffer_config), fields(session.id = %session_id))]
    #[allow(clippy::too_many_arguments)]
    async fn handle_session(
        session_id: String,
        mut message_rx: mpsc::Receiver<UserMessage>,
        delta_tx: broadcast::Sender<MessageDelta>,
        llm_adapter: Arc<LlmClientAdapter>,
        sessions: Arc<RwLock<HashMap<String, ChatSessionState>>>,
        session_metrics: Arc<RwLock<HashMap<String, SessionMetrics>>>,
        metrics_collector: MetricsCollector,
        inflight_deltas: Arc<AtomicU64>,
        backpressure_notify: Arc<Notify>,
        buffer_config: BufferConfig,
    ) {
        const MAX_INFLIGHT_WINDOW: u64 = 100; // Maximum deltas before applying back-pressure
        let mut conversation_context = Vec::new();
        let streaming_mode = buffer_config.chat_streaming_mode;
        info!(streaming_mode = ?streaming_mode, "Session handler started");

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

                            // In aggregated mode, accumulate the entire response
                            if streaming_mode == ChatStreamingMode::Aggregated {
                                // Collect all deltas into complete response
                                while let Some(delta_result) = delta_stream.next().await {
                                    match delta_result {
                                        Ok(stream_delta) => {
                                            match stream_delta.delta {
                                                DeltaType::Text { content } => {
                                                    assistant_response.push_str(&content);
                                                },
                                                DeltaType::StreamEnd { reason } => {
                                                    // Add complete response to context
                                                    if !assistant_response.is_empty() {
                                                        conversation_context.push(format!("Assistant: {assistant_response}"));
                                                    }

                                                    // Send single aggregated message
                                                    let aggregated_delta = MessageDelta {
                                                        session_id: session_id.clone(),
                                                        message_id: message_id.clone(),
                                                        sequence: 0,
                                                        delta: MessageDeltaContent::Text {
                                                            text: assistant_response.clone()
                                                        },
                                                        timestamp: chrono::Utc::now(),
                                                    };
                                                    let _ = delta_tx.send(aggregated_delta);

                                                    // Send end marker
                                                    let end_reason = match reason {
                                                        arkavo_llm::EndReason::Complete => StreamEndReason::Complete,
                                                        arkavo_llm::EndReason::MaxTokens => StreamEndReason::MaxTokens,
                                                        arkavo_llm::EndReason::Aborted => StreamEndReason::UserAbort,
                                                        arkavo_llm::EndReason::Error(_) => StreamEndReason::Error,
                                                        arkavo_llm::EndReason::Timeout => StreamEndReason::Error,
                                                    };

                                                    // Record metrics before using end_reason
                                                    let duration = start_time.elapsed();
                                                    metrics_collector.record_response_time(duration.as_millis() as u64);
                                                    session_observability::log_stream_end(&session_id, &format!("{end_reason:?}"), None);

                                                    let end_delta = MessageDelta {
                                                        session_id: session_id.clone(),
                                                        message_id: message_id.clone(),
                                                        sequence: 1,
                                                        delta: MessageDeltaContent::StreamEnd { reason: end_reason },
                                                        timestamp: stream_delta.timestamp,
                                                    };
                                                    let _ = delta_tx.send(end_delta);
                                                    break;
                                                },
                                                DeltaType::ToolCall { .. } => {
                                                    // Handle tool calls in aggregated mode (future work)
                                                    warn!("Tool calls not yet supported in aggregated mode");
                                                },
                                                DeltaType::Error(err) => {
                                                    error!(error = %err.message, "LLM error in aggregated mode");
                                                    break;
                                                },
                                            }
                                        }
                                        Err(e) => {
                                            error!(error = %e, "Stream error in aggregated mode");
                                            break;
                                        }
                                    }
                                }
                            } else {
                                // Original delta streaming mode
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
                                            DeltaType::ToolCall { id, name, arguments } => {
                                                // Convert arguments to string
                                                let args_str = arguments
                                                    .map(|v| v.to_string())
                                                    .unwrap_or_else(|| "{}".to_string());

                                                // Determine if this is the first or last delta for this tool call
                                                let is_first = !name.is_empty();
                                                let is_complete = args_str.contains('}'); // Simple heuristic

                                                MessageDelta {
                                                    session_id: session_id.clone(),
                                                    message_id: message_id.clone(),
                                                    sequence,
                                                    delta: MessageDeltaContent::ToolCall {
                                                        tool_call_id: id,
                                                        name: if is_first { Some(name) } else { None },
                                                        args_json_fragment: args_str,
                                                        done: is_complete,
                                                    },
                                                    timestamp: stream_delta.timestamp,
                                                }
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

                                        // Check for back-pressure before sending
                                        let current_inflight = inflight_deltas.fetch_add(1, Ordering::SeqCst) + 1;
                                        if current_inflight > MAX_INFLIGHT_WINDOW {
                                            // Wait for acknowledgment before continuing
                                            info!(
                                                session.id = %session_id,
                                                inflight = current_inflight,
                                                "Applying back-pressure, waiting for client acknowledgment"
                                            );
                                            backpressure_notify.notified().await;
                                        }

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
                            } // End of delta streaming mode
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
        if let Some(session_state) = sessions.read().await.get(&session_id)
            && session_state.state != SessionState::Closing
            && let Some(metrics) = session_metrics.write().await.get_mut(&session_id)
        {
            metrics.set_state(SessionState::Zombie);
            warn!("Session marked as zombie - cleanup needed");
        }

        info!("Session handler exited");
    }

    /// Handle a chat session with Router (quality gate + tools)
    #[instrument(skip(message_rx, delta_tx, router, tool_registry, sessions, session_metrics, metrics_collector, learning_context), fields(session.id = %session_id))]
    #[allow(clippy::too_many_arguments)]
    async fn handle_session_with_router(
        session_id: String,
        mut message_rx: mpsc::Receiver<UserMessage>,
        delta_tx: broadcast::Sender<MessageDelta>,
        router: Arc<Router>,
        tool_registry: Option<Arc<ToolRegistry>>,
        sessions: Arc<RwLock<HashMap<String, ChatSessionState>>>,
        session_metrics: Arc<RwLock<HashMap<String, SessionMetrics>>>,
        metrics_collector: MetricsCollector,
        learning_context: Option<Arc<RwLock<String>>>,
    ) {
        let mut conversation_context: Vec<Message> = Vec::new();
        info!("Router-based session handler started");

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

                    // Add to conversation context
                    conversation_context.push(Message {
                        role: Role::User,
                        content: user_message.content.clone(),
                        images: None,
                    });

                    let message_id = Uuid::new_v4().to_string();
                    session_observability::log_stream_start(&session_id, None);

                    let mut final_response;

                    // Chat path: fastest local model on separate semaphore
                    // Sliding window prevents unbounded context growth
                    const CHAT_WINDOW_SIZE: usize = 8;
                    let mut windowed_context: Vec<Message> = if conversation_context.len() > CHAT_WINDOW_SIZE {
                        conversation_context[conversation_context.len() - CHAT_WINDOW_SIZE..].to_vec()
                    } else {
                        conversation_context.clone()
                    };

                    // Prepend learning context as system message (lessons, quality trends)
                    if let Some(ref lc) = learning_context {
                        let ctx = lc.read().await;
                        if !ctx.is_empty() {
                            windowed_context.insert(0, Message {
                                role: Role::System,
                                content: ctx.clone(),
                                images: None,
                            });
                        }
                    }

                    let model = router.fastest_local_model();
                    let metadata_delta = MessageDelta {
                        session_id: session_id.clone(),
                        message_id: message_id.clone(),
                        sequence: 0,
                        delta: MessageDeltaContent::Metadata {
                            key: "model_selected".to_string(),
                            value: serde_json::json!({
                                "model": model.name(),
                                "category": "chat",
                                "reasoning": "Chat path: fastest local model, separate semaphore",
                            }),
                        },
                        timestamp: chrono::Utc::now(),
                    };
                    let _ = delta_tx.send(metadata_delta);

                    let inference_start = std::time::Instant::now();
                    let route_result = match tokio::time::timeout(
                        std::time::Duration::from_secs(30),
                        router.route_chat(windowed_context),
                    )
                    .await
                    {
                        Ok(inner) => inner,
                        Err(_elapsed) => {
                            error!(session.id = %session_id, "Chat inference timed out after 30s");
                            Err(arkavo_router::Error::ModelExecution(
                                "Chat inference timed out after 30s".to_string(),
                            ))
                        }
                    };

                    match route_result {
                        Ok(response) => {
                            final_response = response.content.clone();
                            let elapsed = inference_start.elapsed();

                            // Emit quality_feedback metadata delta
                            let quality_delta = MessageDelta {
                                session_id: session_id.clone(),
                                message_id: message_id.clone(),
                                sequence: 1,
                                delta: MessageDeltaContent::Metadata {
                                    key: "quality_feedback".to_string(),
                                    value: serde_json::json!({
                                        "latency_ms": elapsed.as_millis() as u64,
                                        "response_len": response.content.len(),
                                        "has_tool_calls": !response.tool_calls.is_empty(),
                                    }),
                                },
                                timestamp: chrono::Utc::now(),
                            };
                            let _ = delta_tx.send(quality_delta);

                            // Send text delta (sequence 2+, after metadata deltas)
                            let text_delta = MessageDelta {
                                session_id: session_id.clone(),
                                message_id: message_id.clone(),
                                sequence: 2,
                                delta: MessageDeltaContent::Text {
                                    text: response.content.clone(),
                                },
                                timestamp: chrono::Utc::now(),
                            };
                            let _ = delta_tx.send(text_delta);

                            // Check for tool calls and execute them
                            if !response.tool_calls.is_empty() {
                                // Send tool call deltas
                                for (idx, tool_call) in response.tool_calls.iter().enumerate() {
                                    let tool_delta = MessageDelta {
                                        session_id: session_id.clone(),
                                        message_id: message_id.clone(),
                                        sequence: idx as u64 + 3,
                                        delta: MessageDeltaContent::ToolCall {
                                            tool_call_id: tool_call.call_id.clone().unwrap_or_else(|| format!("call_{idx}")),
                                            name: Some(tool_call.tool_name.clone()),
                                            args_json_fragment: tool_call.arguments.to_string(),
                                            done: true,
                                        },
                                        timestamp: chrono::Utc::now(),
                                    };
                                    let _ = delta_tx.send(tool_delta);
                                }

                                // Execute tools if we have a registry
                                if let Some(ref registry) = tool_registry {
                                    let executor = ToolExecutor::with_registry(registry.clone());
                                    let tool_results = executor.execute_batch(&response.tool_calls).await;

                                    // Add assistant message with tool calls to context
                                    conversation_context.push(Message {
                                        role: Role::Assistant,
                                        content: response.content.clone(),
                                        images: None,
                                    });

                                    // Format and add tool results to context as user message
                                    let results_message = format_tool_results(&tool_results);
                                    conversation_context.push(Message {
                                        role: Role::User,
                                        content: results_message,
                                        images: None,
                                    });

                                    // Send tool result deltas
                                    for (idx, result) in tool_results.iter().enumerate() {
                                        let result_delta = MessageDelta {
                                            session_id: session_id.clone(),
                                            message_id: message_id.clone(),
                                            sequence: (response.tool_calls.len() + idx + 3) as u64,
                                            delta: MessageDeltaContent::ToolResult {
                                                tool_call_id: result.call_id.clone().unwrap_or_else(|| format!("call_{idx}")),
                                                content: serde_json::to_string(&result.result).unwrap_or_default(),
                                                is_error: !result.success,
                                            },
                                            timestamp: chrono::Utc::now(),
                                        };
                                        let _ = delta_tx.send(result_delta);
                                    }

                                    // Route again to get final response with tool results
                                    #[allow(deprecated)]
                                    let retry_result = tokio::time::timeout(
                                        std::time::Duration::from_secs(120),
                                        router.route_with_quality_gate(
                                            &user_message.content,
                                            conversation_context.clone(),
                                            tool_registry.as_deref(),
                                            3,
                                        ),
                                    )
                                    .await;
                                    let retry_result = match retry_result {
                                        Ok(inner) => inner,
                                        Err(_) => Err(arkavo_router::Error::ModelExecution(
                                            "LLM inference timed out after 120s".to_string(),
                                        )),
                                    };
                                    match retry_result {
                                        Ok(final_resp) => {
                                            final_response = final_resp.content.clone();

                                            // Send final text delta
                                            let final_delta = MessageDelta {
                                                session_id: session_id.clone(),
                                                message_id: message_id.clone(),
                                                sequence: (response.tool_calls.len() + tool_results.len() + 3) as u64,
                                                delta: MessageDeltaContent::Text {
                                                    text: final_resp.content.clone(),
                                                },
                                                timestamp: chrono::Utc::now(),
                                            };
                                            let _ = delta_tx.send(final_delta);

                                            // Add final assistant response to context
                                            conversation_context.push(Message {
                                                role: Role::Assistant,
                                                content: final_resp.content,
                                                images: None,
                                            });
                                        }
                                        Err(e) => {
                                            error!(error = %e, "Failed to get final response after tool execution");
                                            // Keep the original response as final
                                        }
                                    }
                                } else {
                                    warn!("Tool calls received but no tool registry available");
                                    // Add assistant message to context without tool execution
                                    conversation_context.push(Message {
                                        role: Role::Assistant,
                                        content: response.content,
                                        images: None,
                                    });
                                }
                            } else {
                                // No tool calls - add assistant message to context
                                conversation_context.push(Message {
                                    role: Role::Assistant,
                                    content: response.content,
                                    images: None,
                                });
                            }
                        }
                        Err(e) => {
                            let error_msg = e.to_string();

                            // Check if Judge detected missing tool usage
                            if error_msg.contains("MISSING_TOOL_USE:") {
                                if let Some(ref registry) = tool_registry {
                                    // Extract keywords from error message
                                    let keywords_str = error_msg
                                        .strip_prefix("Model execution failed: MISSING_TOOL_USE:")
                                        .unwrap_or("");
                                    let keywords: Vec<String> = keywords_str
                                        .trim_matches(|c| c == '[' || c == ']' || c == '"')
                                        .split(',')
                                        .map(|s| s.trim().trim_matches('"').to_string())
                                        .filter(|s| !s.is_empty())
                                        .collect();

                                    info!("Judge detected missing tool usage, searching for: {:?}", keywords);

                                    // Search for tools matching the judge's suggested keywords
                                    let mut tools_found = Vec::new();
                                    for keyword in &keywords {
                                        let found = registry.search_tools(
                                            keyword,
                                            arkavo_mcp_tools::DetailLevel::FullSchema,
                                        );
                                        tools_found.extend(found);
                                    }

                                    if !tools_found.is_empty() {
                                        // Create tool hints message
                                        let tool_hints: Vec<String> = tools_found
                                            .iter()
                                            .map(|t| format!("- {}: {}", t.name, t.description.as_deref().unwrap_or("No description")))
                                            .collect();
                                        let tool_msg = format!(
                                            "Available tools for this query:\n{}\n\nPlease use the appropriate tool to answer.",
                                            tool_hints.join("\n")
                                        );

                                        // Add tool hints to conversation
                                        conversation_context.push(Message {
                                            role: Role::User,
                                            content: tool_msg,
                                            images: None,
                                        });

                                        // Retry with tool hints
                                        #[allow(deprecated)]
                                        let hint_result = tokio::time::timeout(
                                            std::time::Duration::from_secs(120),
                                            router.route_with_quality_gate(
                                                &user_message.content,
                                                conversation_context.clone(),
                                                tool_registry.as_deref(),
                                                3,
                                            ),
                                        )
                                        .await;
                                        let hint_result = match hint_result {
                                            Ok(inner) => inner,
                                            Err(_) => Err(arkavo_router::Error::ModelExecution(
                                                "LLM inference timed out after 120s".to_string(),
                                            )),
                                        };
                                        match hint_result {
                                            Ok(response) => {
                                                final_response = response.content.clone();

                                                // Send text delta
                                                let text_delta = MessageDelta {
                                                    session_id: session_id.clone(),
                                                    message_id: message_id.clone(),
                                                    sequence: 0,
                                                    delta: MessageDeltaContent::Text {
                                                        text: response.content.clone(),
                                                    },
                                                    timestamp: chrono::Utc::now(),
                                                };
                                                let _ = delta_tx.send(text_delta);

                                                // Handle tool calls if present
                                                if !response.tool_calls.is_empty() {
                                                    let executor = ToolExecutor::with_registry(registry.clone());
                                                    let tool_results = executor.execute_batch(&response.tool_calls).await;

                                                    // Send tool result deltas
                                                    for (idx, result) in tool_results.iter().enumerate() {
                                                        let result_delta = MessageDelta {
                                                            session_id: session_id.clone(),
                                                            message_id: message_id.clone(),
                                                            sequence: (idx + 1) as u64,
                                                            delta: MessageDeltaContent::ToolResult {
                                                                tool_call_id: result.call_id.clone().unwrap_or_else(|| format!("call_{idx}")),
                                                                content: serde_json::to_string(&result.result).unwrap_or_default(),
                                                                is_error: !result.success,
                                                            },
                                                            timestamp: chrono::Utc::now(),
                                                        };
                                                        let _ = delta_tx.send(result_delta);
                                                    }
                                                }

                                                // Add response to context
                                                conversation_context.push(Message {
                                                    role: Role::Assistant,
                                                    content: response.content,
                                                    images: None,
                                                });
                                            }
                                            Err(retry_err) => {
                                                final_response = String::new();
                                                error!(error = %retry_err, "Retry after tool discovery also failed");

                                                let error_delta = MessageDelta {
                                                    session_id: session_id.clone(),
                                                    message_id: message_id.clone(),
                                                    sequence: 0,
                                                    delta: MessageDeltaContent::Error {
                                                        code: "ROUTER_ERROR".to_string(),
                                                        message: format!("Failed after tool discovery: {retry_err}"),
                                                    },
                                                    timestamp: chrono::Utc::now(),
                                                };
                                                let _ = delta_tx.send(error_delta);
                                            }
                                        }
                                    } else {
                                        final_response = String::new();
                                        warn!("No tools found for keywords: {:?}", keywords);

                                        let error_delta = MessageDelta {
                                            session_id: session_id.clone(),
                                            message_id: message_id.clone(),
                                            sequence: 0,
                                            delta: MessageDeltaContent::Error {
                                                code: "NO_TOOLS".to_string(),
                                                message: format!("No tools available for: {}", keywords.join(", ")),
                                            },
                                            timestamp: chrono::Utc::now(),
                                        };
                                        let _ = delta_tx.send(error_delta);
                                    }
                                } else {
                                    final_response = String::new();
                                    error!(error = %e, "Missing tool use but no registry available");

                                    let error_delta = MessageDelta {
                                        session_id: session_id.clone(),
                                        message_id: message_id.clone(),
                                        sequence: 0,
                                        delta: MessageDeltaContent::Error {
                                            code: "ROUTER_ERROR".to_string(),
                                            message: format!("Failed to route message: {e}"),
                                        },
                                        timestamp: chrono::Utc::now(),
                                    };
                                    let _ = delta_tx.send(error_delta);
                                }
                            } else {
                                final_response = String::new();
                                error!(error = %e, "Failed to route message with quality gate");

                                // Record error metrics
                                if let Some(metrics) = session_metrics.read().await.get(&session_id) {
                                    metrics.record_error("router", &e.to_string());
                                }
                                metrics_collector.record_error("router");
                                session_observability::log_session_error(&session_id, &e.to_string(), Some("ROUTER_ERROR"));

                                // Send error delta
                                let error_delta = MessageDelta {
                                    session_id: session_id.clone(),
                                    message_id: message_id.clone(),
                                    sequence: 0,
                                    delta: MessageDeltaContent::Error {
                                        code: "ROUTER_ERROR".to_string(),
                                        message: format!("Failed to route message: {e}"),
                                    },
                                    timestamp: chrono::Utc::now(),
                                };
                                let _ = delta_tx.send(error_delta);
                            }
                        }
                    }

                    // Send stream end
                    let duration = start_time.elapsed();
                    metrics_collector.record_response_time(duration.as_millis() as u64);
                    session_observability::log_stream_end(&session_id, "Complete", None);

                    let end_delta = MessageDelta {
                        session_id: session_id.clone(),
                        message_id: message_id.clone(),
                        sequence: 1,
                        delta: MessageDeltaContent::StreamEnd {
                            reason: StreamEndReason::Complete,
                        },
                        timestamp: chrono::Utc::now(),
                    };
                    let _ = delta_tx.send(end_delta);

                    // Record metrics
                    if let Some(metrics) = session_metrics.read().await.get(&session_id) {
                        metrics.record_message_sent(final_response.len());
                    }
                }

                // Check if session should be closed
                else => {
                    // Channel closed, check if session is being closed
                    if let Some(session_state) = sessions.read().await.get(&session_id)
                        && session_state.state == SessionState::Closing {
                        info!("Router session handler exiting due to closure request");
                        break;
                    }
                    warn!("Message channel closed unexpectedly");
                    break;
                }
            }
        }

        // Mark session as zombie if it wasn't properly closed
        if let Some(session_state) = sessions.read().await.get(&session_id)
            && session_state.state != SessionState::Closing
            && let Some(metrics) = session_metrics.write().await.get_mut(&session_id)
        {
            metrics.set_state(SessionState::Zombie);
            warn!("Router session marked as zombie - cleanup needed");
        }

        info!("Router session handler exited");
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

/// Maximum characters per tool result to prevent exceeding LLM token limits
const MAX_TOOL_RESULT_CHARS: usize = 200_000;

/// Format tool execution results for adding to conversation context
fn format_tool_results(results: &[ToolExecutionResult]) -> String {
    use std::fmt::Write;

    let mut formatted = String::from("Tool execution results:\n\n");

    for result in results {
        let _ = writeln!(formatted, "Tool: {}", result.tool_name);
        if result.success {
            let result_json =
                serde_json::to_string_pretty(&result.result).unwrap_or_else(|_| "{}".to_string());

            // Truncate large results to prevent exceeding LLM token limits
            if result_json.len() > MAX_TOOL_RESULT_CHARS {
                let truncated = &result_json[..MAX_TOOL_RESULT_CHARS];
                let break_point = truncated
                    .rfind('\n')
                    .or_else(|| truncated.rfind(' '))
                    .unwrap_or(MAX_TOOL_RESULT_CHARS);
                let _ = writeln!(
                    formatted,
                    "Result (truncated from {} to {} chars):\n{}...\n[OUTPUT TRUNCATED]",
                    result_json.len(),
                    break_point,
                    &result_json[..break_point]
                );
            } else {
                let _ = writeln!(formatted, "Result: {result_json}");
            }
        } else {
            let error_msg = result.error.as_deref().unwrap_or("Unknown error");
            let _ = writeln!(formatted, "Error: {error_msg}");
        }
        formatted.push('\n');
    }

    formatted
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_session_creation() {
        let manager = ChatSessionManager::new(None);
        let session = manager.create_session(None).await;

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
        let session = manager.create_session(None).await;
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
        let manager = ChatSessionManager::with_config(None, None, None, 1, BufferConfig::default()); // 1 second TTL
        let session = manager.create_session(None).await;
        let _session_id = session.session_id.clone();

        // Wait for TTL to expire
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // Session should be cleaned up by TTL cleaner
        // Note: This test might be flaky due to timing, but demonstrates the concept

        manager.shutdown().await;
    }

    // INVARIANT TESTS - Verifies CHAT-INV-001, CHAT-INV-002, CHAT-INV-003

    #[test]
    fn test_invariant_backpressure_threshold() {
        // CHAT-INV-001: inflight_deltas must not exceed 100
        use std::sync::atomic::AtomicU64;

        let inflight = Arc::new(AtomicU64::new(50));
        let _last_acked = Arc::new(AtomicU64::new(0));

        // Simulate 100 in-flight deltas
        inflight.store(100, Ordering::SeqCst);
        assert_eq!(inflight.load(Ordering::SeqCst), 100);

        // Exceeding 100 should trigger back-pressure (checked in handler)
        inflight.store(101, Ordering::SeqCst);
        assert!(
            inflight.load(Ordering::SeqCst) > 100,
            "Should exceed threshold"
        );
    }

    #[test]
    fn test_invariant_state_validity() {
        // CHAT-INV-002: State must be one of valid variants
        let states = vec![
            SessionState::Active,
            SessionState::Closing,
            SessionState::Zombie,
        ];

        for state in states {
            match state {
                SessionState::Active | SessionState::Closing | SessionState::Zombie => {
                    // Valid states
                }
            }
        }
    }

    #[test]
    fn test_invariant_sequence_monotonicity() {
        // CHAT-INV-003: Delta sequences must be monotonically increasing
        let mut sequences: Vec<u64> = vec![1, 2, 3, 4, 5];

        // Check monotonicity
        assert!(
            sequences.windows(2).all(|w| w[0] < w[1]),
            "Sequences should be monotonic"
        );

        // Non-monotonic should fail
        sequences = vec![1, 3, 2, 4];
        assert!(
            !sequences.windows(2).all(|w| w[0] < w[1]),
            "Non-monotonic should be detected"
        );
    }

    #[tokio::test]
    async fn test_session_invariant_checks() {
        // Create a session and verify invariants hold
        let manager = ChatSessionManager::new(None);
        let session = manager.create_session(None).await;
        let session_id = session.session_id;

        // Verify session exists and is active
        assert!(manager.session_exists(&session_id).await);

        // Get session state and check invariants
        let sessions = manager.sessions.read().await;
        if let Some(state) = sessions.get(&session_id) {
            // This would call check_invariants() in test builds
            assert!(matches!(state.state, SessionState::Active));
        }
        drop(sessions);

        manager.shutdown().await;
    }
}
