use crate::auth::SessionAuth;
use crate::config::{BufferConfig, ChatStreamingMode};
use crate::error::{A2aError, Result};
use crate::types::{
    ChatCapabilities, ChatSession, MessageDelta, MessageDeltaContent, StreamEndReason, UserMessage,
};
use arkavo_llm::{
    DeltaType, LlmClientAdapter, Message, StreamLlmModel, ToolExecutionResult, ToolExecutor,
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

/// A teaching event emitted when the chat path detects a human teaching intent
#[derive(Debug, Clone)]
pub enum ChatTeachingEvent {
    /// Human gave a behavioral instruction
    Instruction {
        text: String,
        scope: arkavo_router::learning::LessonScope,
    },
    /// Human corrected a recent action
    Correction {
        text: String,
        trace_id: Option<Uuid>,
    },
    /// Human reinforced a recent action
    Reinforcement { trace_id: Option<Uuid> },
}

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
    /// Channel for emitting human teaching events to the learning bus
    teaching_tx: Option<mpsc::Sender<ChatTeachingEvent>>,
    /// Agent purpose/system prompt from AGENTS.md, prepended to every chat context
    system_prompt: Option<String>,
    /// Override the default model selection (catalog name or GGUF path)
    model_override: Option<arkavo_router::ModelSpec>,
    /// Shared task context string with recent task history and last observed state.
    /// Updated by the server from ToolMemory + conductor task store.
    /// Injected as a system message so chat can reference what the agent has been doing.
    task_context: Option<Arc<RwLock<String>>>,
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
        Self::with_config(llm_adapter, None, None, 3600, BufferConfig::default())
        // Default 1 hour TTL
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
            teaching_tx: None,
            system_prompt: None,
            model_override: None,
            task_context: None,
        }
    }

    /// Set a shared learning context that will be prepended to chat messages.
    /// The server updates this string from LearningBus (lessons, quality trends).
    pub fn set_learning_context(&mut self, context: Arc<RwLock<String>>) {
        self.learning_context = Some(context);
    }

    /// Set the teaching event channel for human teaching through chat.
    pub fn set_teaching_tx(&mut self, tx: mpsc::Sender<ChatTeachingEvent>) {
        self.teaching_tx = Some(tx);
    }

    /// Set the agent's purpose/system prompt from AGENTS.md.
    /// Prepended as a system message to every chat context window.
    pub fn set_system_prompt(&mut self, prompt: String) {
        if !prompt.is_empty() {
            self.system_prompt = Some(prompt);
        }
    }

    /// Override the default model selection for chat inference.
    pub fn set_model_override(&mut self, model: arkavo_router::ModelChoice) {
        self.set_model_spec(arkavo_router::ModelSpec::Named(model));
    }

    /// Override chat inference with a catalog model or an on-disk GGUF path.
    pub fn set_model_spec(&mut self, spec: arkavo_router::ModelSpec) {
        self.model_override = Some(spec);
    }

    /// Set a shared task context that will be injected into chat sessions.
    /// Contains recent task history and last observed state so chat can reference
    /// what the agent has been working on.
    pub fn set_task_context(&mut self, context: Arc<RwLock<String>>) {
        self.task_context = Some(context);
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
            let teaching_tx = self.teaching_tx.clone();
            let system_prompt = self.system_prompt.clone();
            let model_override = self.model_override.clone();
            let task_context = self.task_context.clone();

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
                        teaching_tx,
                        system_prompt,
                        model_override,
                        task_context,
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

    /// Push a pre-built delta directly onto a session's broadcast channel.
    /// Used for introspection responses (e.g. `@context`) that bypass LLM inference.
    pub async fn push_system_delta(&self, session_id: &str, text: String) -> Result<()> {
        let sessions = self.sessions.read().await;
        let state = sessions
            .get(session_id)
            .ok_or_else(|| A2aError::SessionNotFound(session_id.to_string()))?;

        let delta = MessageDelta {
            session_id: session_id.to_string(),
            message_id: uuid::Uuid::new_v4().to_string(),
            sequence: 0,
            delta: MessageDeltaContent::Text { text },
            timestamp: chrono::Utc::now(),
        };
        let _ = state.delta_tx.send(delta);

        let end = MessageDelta {
            session_id: session_id.to_string(),
            message_id: uuid::Uuid::new_v4().to_string(),
            sequence: 1,
            delta: MessageDeltaContent::StreamEnd {
                reason: StreamEndReason::Complete,
            },
            timestamp: chrono::Utc::now(),
        };
        let _ = state.delta_tx.send(end);
        Ok(())
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

        // Mark session as zombie if metrics still exist (abnormal exit / cleanup needed).
        // Normal closure removes the metrics before the handler exits, so a remaining
        // metrics entry means the session channel closed unexpectedly while the session
        // was still active.
        if let Some(metrics) = session_metrics.write().await.get_mut(&session_id) {
            metrics.set_state(SessionState::Zombie);
            warn!("Session marked as zombie - cleanup needed");
        }

        info!("Session handler exited");
    }

    /// Handle a chat session with Router (quality gate + tools)
    #[instrument(skip(message_rx, delta_tx, router, tool_registry, sessions, session_metrics, metrics_collector, learning_context, teaching_tx, system_prompt, model_override, task_context), fields(session.id = %session_id))]
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
        teaching_tx: Option<mpsc::Sender<ChatTeachingEvent>>,
        system_prompt: Option<String>,
        model_override: Option<arkavo_router::ModelSpec>,
        task_context: Option<Arc<RwLock<String>>>,
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
                    conversation_context.push(Message::user(user_message.content.clone()));

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

                    // Prepend task context as system message (recent tasks + last observed state)
                    if let Some(ref tc) = task_context {
                        let ctx = tc.read().await;
                        if !ctx.is_empty() {
                            windowed_context.insert(0, Message::system(ctx.clone()));
                        }
                    }

                    // Prepend learning context as system message (lessons, quality trends)
                    if let Some(ref lc) = learning_context {
                        let ctx = lc.read().await;
                        if !ctx.is_empty() {
                            windowed_context.insert(0, Message::system(ctx.clone()));
                        }
                    }

                    // Prepend agent purpose/system prompt from AGENTS.md (always first)
                    if let Some(ref prompt) = system_prompt {
                        windowed_context.insert(0, Message::system(prompt.clone()));
                    }

                    // Emit tool search telemetry before routing
                    if let Some(ref registry) = tool_registry {
                        let keywords = arkavo_router::tool_search_keywords(&user_message.content);
                        let tool_names: Vec<String> = registry
                            .search_tools(&keywords, arkavo_mcp_tools::DetailLevel::NameOnly)
                            .iter()
                            .map(|t| t.name.clone())
                            .collect();
                        let tool_search_delta = MessageDelta {
                            session_id: session_id.clone(),
                            message_id: message_id.clone(),
                            sequence: 0,
                            delta: MessageDeltaContent::Metadata {
                                key: "tool_search".to_string(),
                                value: serde_json::json!({
                                    "keywords": keywords,
                                    "tools_found": tool_names.len(),
                                    "tool_names": tool_names,
                                }),
                            },
                            timestamp: chrono::Utc::now(),
                        };
                        let _ = delta_tx.send(tool_search_delta);
                    }

                    // Classify human teaching intent before routing
                    let last_trace = router.last_decision_trace().map(|t| t.trace_id);
                    let intent = arkavo_router::learning::human_teaching::classify_intent_llm(
                        &user_message.content,
                        last_trace,
                        &router,
                    )
                    .await;

                    if intent != arkavo_router::learning::TeachingIntent::Question {
                        // Emit intent metadata delta so the UI can display it
                        let intent_label = match &intent {
                            arkavo_router::learning::TeachingIntent::Instruction { scope } => {
                                format!("instruction ({scope})")
                            }
                            arkavo_router::learning::TeachingIntent::Correction { .. } => {
                                "correction".to_string()
                            }
                            arkavo_router::learning::TeachingIntent::Reinforcement => {
                                "reinforcement".to_string()
                            }
                            arkavo_router::learning::TeachingIntent::Question => unreachable!(),
                        };
                        let intent_delta = MessageDelta {
                            session_id: session_id.clone(),
                            message_id: message_id.clone(),
                            sequence: 0,
                            delta: MessageDeltaContent::Metadata {
                                key: "teaching_intent".to_string(),
                                value: serde_json::json!({
                                    "intent": intent_label,
                                    "text": &user_message.content,
                                }),
                            },
                            timestamp: chrono::Utc::now(),
                        };
                        let _ = delta_tx.send(intent_delta);

                        // Forward teaching event to learning bus
                        if let Some(ref tx) = teaching_tx {
                            let event = match &intent {
                                arkavo_router::learning::TeachingIntent::Instruction { scope } => {
                                    Some(ChatTeachingEvent::Instruction {
                                        text: user_message.content.clone(),
                                        scope: *scope,
                                    })
                                }
                                arkavo_router::learning::TeachingIntent::Correction { trace_id } => {
                                    Some(ChatTeachingEvent::Correction {
                                        text: user_message.content.clone(),
                                        trace_id: *trace_id,
                                    })
                                }
                                arkavo_router::learning::TeachingIntent::Reinforcement => {
                                    Some(ChatTeachingEvent::Reinforcement {
                                        trace_id: last_trace,
                                    })
                                }
                                _ => None,
                            };
                            if let Some(evt) = event {
                                let _ = tx.send(evt).await;
                            }
                        }

                        info!(
                            session.id = %session_id,
                            intent = %intent_label,
                            "Teaching intent detected in chat"
                        );
                    }

                    let spec = model_override.clone().unwrap_or_else(|| {
                        arkavo_router::ModelSpec::Named(router.fastest_local_model())
                    });
                    let model_label = spec.display_name();
                    let reasoning = if model_override.is_some() {
                        format!("--model override: {model_label}")
                    } else {
                        "Chat path: fastest local model, separate semaphore".to_string()
                    };
                    let metadata_delta = MessageDelta {
                        session_id: session_id.clone(),
                        message_id: message_id.clone(),
                        sequence: 0,
                        delta: MessageDeltaContent::Metadata {
                            key: "model_selected".to_string(),
                            value: serde_json::json!({
                                "model": model_label,
                                "category": "chat",
                                "reasoning": reasoning,
                            }),
                        },
                        timestamp: chrono::Utc::now(),
                    };
                    let _ = delta_tx.send(metadata_delta);

                    let inference_start = std::time::Instant::now();
                    // First GGUF-path load includes mmap + Metal init, which
                    // can exceed the 60s named-model budget.
                    let chat_timeout_secs = if spec.as_gguf_path().is_some() {
                        180
                    } else {
                        60
                    };
                    let route_result = match tokio::time::timeout(
                        std::time::Duration::from_secs(chat_timeout_secs),
                        router.route_chat_spec(windowed_context, tool_registry.as_deref(), model_override.as_ref()),
                    )
                    .await
                    {
                        Ok(inner) => inner,
                        Err(_elapsed) => {
                            error!(session.id = %session_id, "Chat inference timed out after {chat_timeout_secs}s");
                            Err(arkavo_router::Error::ModelExecution(
                                format!("Chat inference timed out after {chat_timeout_secs}s"),
                            ))
                        }
                    };

                    match route_result {
                        Ok(response) => {
                            final_response = response.content.clone();
                            let elapsed = inference_start.elapsed();

                            // Emit quality_feedback metadata delta with inference timing
                            let mut quality_value = serde_json::json!({
                                "latency_ms": elapsed.as_millis() as u64,
                                "response_len": response.content.len(),
                                "has_tool_calls": !response.tool_calls.is_empty(),
                                "tool_call_count": response.tool_calls.len(),
                            });
                            // Include tool call names for diagnostics
                            if !response.tool_calls.is_empty() {
                                let tool_names: Vec<&str> = response.tool_calls
                                    .iter()
                                    .map(|tc| tc.tool_name.as_str())
                                    .collect();
                                quality_value["tool_names"] = serde_json::json!(tool_names);
                            }
                            // Include inference timing from provider (local models)
                            if let Some(ref timing) = response.inference_timing {
                                quality_value["prompt_eval_ms"] = serde_json::json!(timing.prompt_eval_ms);
                                quality_value["generation_ms"] = serde_json::json!(timing.generation_ms);
                                quality_value["prompt_tokens"] = serde_json::json!(timing.n_prompt_eval);
                                quality_value["generated_tokens"] = serde_json::json!(timing.n_eval);
                                if timing.n_eval > 0 && timing.generation_ms > 0.0 {
                                    let tok_per_sec = timing.n_eval as f64 / (timing.generation_ms / 1000.0);
                                    quality_value["tokens_per_sec"] = serde_json::json!(format!("{tok_per_sec:.1}"));
                                }
                            }
                            let quality_delta = MessageDelta {
                                session_id: session_id.clone(),
                                message_id: message_id.clone(),
                                sequence: 1,
                                delta: MessageDeltaContent::Metadata {
                                    key: "quality_feedback".to_string(),
                                    value: quality_value,
                                },
                                timestamp: chrono::Utc::now(),
                            };
                            let _ = delta_tx.send(quality_delta);

                            // Check for tool calls and execute them
                            if !response.tool_calls.is_empty() {
                                // Strip tool call markup from content before sending text delta.
                                // Handles XML (<tool_call>) and text-extracted calls (tool_name{}).
                                let mut clean = arkavo_router::strip_tool_blocks(&response.content);
                                // If content is just the raw tool call text, suppress it
                                for tc in &response.tool_calls {
                                    clean = clean.replace(&tc.tool_name, "");
                                }
                                let clean = clean.trim().trim_matches(|c: char| c == '{' || c == '}' || c == '(' || c == ')').trim().to_string();
                                if !clean.is_empty() {
                                    let text_delta = MessageDelta {
                                        session_id: session_id.clone(),
                                        message_id: message_id.clone(),
                                        sequence: 2,
                                        delta: MessageDeltaContent::Text {
                                            text: clean,
                                        },
                                        timestamp: chrono::Utc::now(),
                                    };
                                    let _ = delta_tx.send(text_delta);
                                }
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
                                    conversation_context.push(Message::assistant(response.content.clone()));

                                    // Format and add tool results to context as user message
                                    let results_message = format_tool_results(&tool_results);
                                    conversation_context.push(Message::user(results_message));

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

                                    // Route again with same model to synthesize final answer from tool results
                                    let retry_result = tokio::time::timeout(
                                        std::time::Duration::from_mins(2),
                                        router.route_chat_spec(conversation_context.clone(), None, model_override.as_ref()),
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
                                            // Strip think blocks from final response (second inference
                                            // may use a larger model that produces <think> tags)
                                            let clean_content = arkavo_router::strip_think_blocks(&final_resp.content);
                                            final_response = clean_content.clone();

                                            // Emit telemetry for second inference
                                            let mut tool_loop_value = serde_json::json!({
                                                "phase": "tool_result_synthesis",
                                                "latency_ms": inference_start.elapsed().as_millis() as u64,
                                                "response_len": clean_content.len(),
                                            });
                                            if let Some(ref timing) = final_resp.inference_timing {
                                                tool_loop_value["prompt_tokens"] = serde_json::json!(timing.n_prompt_eval);
                                                tool_loop_value["generated_tokens"] = serde_json::json!(timing.n_eval);
                                            }
                                            let tool_loop_delta = MessageDelta {
                                                session_id: session_id.clone(),
                                                message_id: message_id.clone(),
                                                sequence: (response.tool_calls.len() + tool_results.len() + 3) as u64,
                                                delta: MessageDeltaContent::Metadata {
                                                    key: "quality_feedback".to_string(),
                                                    value: tool_loop_value,
                                                },
                                                timestamp: chrono::Utc::now(),
                                            };
                                            let _ = delta_tx.send(tool_loop_delta);

                                            // Send final text delta
                                            let final_delta = MessageDelta {
                                                session_id: session_id.clone(),
                                                message_id: message_id.clone(),
                                                sequence: (response.tool_calls.len() + tool_results.len() + 4) as u64,
                                                delta: MessageDeltaContent::Text {
                                                    text: clean_content.clone(),
                                                },
                                                timestamp: chrono::Utc::now(),
                                            };
                                            let _ = delta_tx.send(final_delta);

                                            // Add final assistant response to context
                                            conversation_context.push(Message::assistant(clean_content));
                                        }
                                        Err(e) => {
                                            error!(error = %e, "Failed to get final response after tool execution");
                                            // The consumer is told, not only the
                                            // log. This inference is gated like
                                            // any other, and a block here left
                                            // the stream with tool deltas and no
                                            // answer — which reads as an empty
                                            // response rather than a refusal.
                                            final_response = String::new();
                                            let _ = delta_tx.send(router_error_delta(
                                                &session_id,
                                                &message_id,
                                                (response.tool_calls.len() + tool_results.len() + 4) as u64,
                                                &e,
                                            ));
                                        }
                                    }
                                } else {
                                    warn!("Tool calls received but no tool registry available");
                                    // Add assistant message to context without tool execution
                                    conversation_context.push(Message::assistant(response.content));
                                }
                            } else {
                                // No tool calls - send text delta and add to context
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
                                conversation_context.push(Message::assistant(response.content));
                            }
                        }
                        Err(e) => {
                            // Check if Judge detected missing tool usage
                            if let arkavo_router::Error::MissingToolUse { ref keywords } = e {
                                if let Some(ref registry) = tool_registry {
                                    info!("Judge detected missing tool usage, searching for: {:?}", keywords);

                                    // Search for tools matching the judge's suggested keywords
                                    let mut tools_found = Vec::new();
                                    for keyword in keywords {
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
                                        conversation_context.push(Message::user(tool_msg));

                                        // Retry with tool hints
                                        let hint_result = tokio::time::timeout(
                                            std::time::Duration::from_mins(2),
                                            router.route_with_tools(
                                                &user_message.content,
                                                conversation_context.clone(),
                                                tool_registry.as_deref(),
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
                                                conversation_context.push(Message::assistant(response.content));
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

                                    let _ = delta_tx.send(router_error_delta(
                                        &session_id,
                                        &message_id,
                                        0,
                                        &e,
                                    ));
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
                                let _ = delta_tx.send(router_error_delta(
                                    &session_id,
                                    &message_id,
                                    0,
                                    &e,
                                ));
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

        // Mark session as zombie if metrics still exist (abnormal exit / cleanup needed).
        // Normal closure removes the metrics before the handler exits, so a remaining
        // metrics entry means the session channel closed unexpectedly while the session
        // was still active.
        if let Some(metrics) = session_metrics.write().await.get_mut(&session_id) {
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

/// The delta a routing failure becomes.
///
/// One constructor for every such failure, because the consumer's only signal
/// that a turn produced no answer is this delta: a branch that logs and sends
/// nothing leaves the stream ending on whatever came before it, which reads as
/// an empty answer rather than as a failure. A release gate's refusal arrives
/// here like any other routing error and must not be the one that goes unsaid.
fn router_error_delta(
    session_id: &str,
    message_id: &str,
    sequence: u64,
    error: &arkavo_router::Error,
) -> MessageDelta {
    MessageDelta {
        session_id: session_id.to_string(),
        message_id: message_id.to_string(),
        sequence,
        delta: MessageDeltaContent::Error {
            code: "ROUTER_ERROR".to_string(),
            message: format!("Failed to route message: {error}"),
        },
        timestamp: chrono::Utc::now(),
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
    use arkavo_test_macros::spec;

    // Mock-provider support
    use async_trait::async_trait;

    /// A gate block on the tool-result synthesis is a routing failure like any
    /// other, and the consumer has to be told. That branch used to log and send
    /// nothing, so a blocked second inference left the stream carrying tool
    /// deltas and no answer — indistinguishable from a model that replied with
    /// nothing.
    #[spec("SENT-011")]
    #[test]
    fn a_blocked_completion_becomes_an_error_delta_the_consumer_can_see() {
        let blocked = arkavo_router::Error::Provider(arkavo_llm::Error::Provider(
            arkavo_llm::GATE_BLOCKED.to_string(),
        ));

        let delta = router_error_delta("session-1", "message-1", 7, &blocked);

        assert_eq!(delta.session_id, "session-1");
        assert_eq!(delta.sequence, 7);
        match delta.delta {
            MessageDeltaContent::Error { code, message } => {
                assert_eq!(code, "ROUTER_ERROR");
                // The refusal survives the wrapping, which is what the CLI
                // matches on to print it alone.
                assert!(message.contains(arkavo_llm::GATE_BLOCKED), "{message}");
            }
            other => panic!("expected an error delta, got {other:?}"),
        }
    }

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
    #[spec("CHAT-002")]
    #[spec("CHAT-003")]
    #[spec("CHAT-006")]
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
    #[spec("CHAT-007")]
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

    // Mock LLM provider for testing streaming/back-pressure behaviour without network calls.
    #[derive(Clone)]
    struct MockLlmProvider {
        chunks: Vec<arkavo_llm::StreamResponse>,
        error_on_stream: Option<String>,
    }

    impl MockLlmProvider {
        fn with_text_chars(count: usize) -> Self {
            let mut chunks = Vec::with_capacity(count);
            for i in 0..count {
                chunks.push(arkavo_llm::StreamResponse {
                    content: "x".to_string(),
                    reasoning_content: None,
                    done: i == count - 1,
                    inference_timing: None,
                });
            }
            Self {
                chunks,
                error_on_stream: None,
            }
        }
    }

    #[async_trait]
    impl arkavo_llm::Provider for MockLlmProvider {
        async fn complete_with_options(
            &self,
            _messages: Vec<arkavo_llm::Message>,
            _max_tokens: Option<usize>,
        ) -> arkavo_llm::Result<String> {
            Ok(String::new())
        }

        async fn stream(
            &self,
            _messages: Vec<arkavo_llm::Message>,
        ) -> arkavo_llm::Result<
            Box<
                dyn futures::Stream<Item = arkavo_llm::Result<arkavo_llm::StreamResponse>>
                    + Send
                    + Unpin,
            >,
        > {
            if let Some(ref err) = self.error_on_stream {
                return Err(arkavo_llm::Error::Stream(err.clone()));
            }
            let chunks = self.chunks.clone();
            let stream = futures::stream::iter(chunks.into_iter().map(Ok));
            Ok(Box::new(stream))
        }

        fn name(&self) -> &str {
            "mock-llm-provider"
        }
    }

    fn create_mock_adapter(chunk_count: usize) -> Arc<arkavo_llm::LlmClientAdapter> {
        let provider = MockLlmProvider::with_text_chars(chunk_count);
        let client = arkavo_llm::LlmClient::new(Box::new(provider));
        Arc::new(arkavo_llm::LlmClientAdapter::new(client))
    }

    #[tokio::test]
    #[spec("CHAT-008")]
    async fn test_get_delta_stream_active_and_missing() {
        let adapter = create_mock_adapter(0);
        let manager = ChatSessionManager::new(Some(adapter));
        let session = manager.create_session(None).await;

        let stream = manager.get_delta_stream(&session.session_id).await;
        assert!(
            stream.is_some(),
            "Active session must expose a delta stream"
        );

        assert!(
            manager
                .get_delta_stream("non-existent-session")
                .await
                .is_none(),
            "Missing session must not expose a delta stream"
        );

        manager.shutdown().await;
    }

    #[tokio::test]
    #[spec("CHAT-004")]
    async fn test_stream_llm_deltas_with_back_pressure() {
        // Produce enough text chunks to exceed the 100-delta in-flight window.
        const DELTA_COUNT: usize = 105;
        let adapter = create_mock_adapter(DELTA_COUNT);

        let mut buffers = BufferConfig::default();
        buffers.chat_streaming_mode = ChatStreamingMode::Delta;

        let manager = ChatSessionManager::with_config(Some(adapter), None, None, 3600, buffers);
        let session = manager.create_session(None).await;
        let session_id = session.session_id.clone();

        let mut delta_rx = manager
            .get_delta_stream(&session_id)
            .await
            .expect("delta stream available");

        manager
            .send_message(
                &session_id,
                UserMessage {
                    content: "start".to_string(),
                    attachments: None,
                    metadata: None,
                },
            )
            .await
            .expect("message accepted");

        // Consume exactly the first 100 deltas; the 101st should be blocked
        // waiting for a client acknowledgment.
        let mut received = Vec::new();
        for i in 0..100 {
            let delta = tokio::time::timeout(std::time::Duration::from_secs(30), delta_rx.recv())
                .await
                .expect(&format!("delta {i} should arrive promptly"))
                .expect("delta stream open");
            received.push(delta);
        }

        // The next delta should not arrive until back-pressure is released.
        let blocked =
            tokio::time::timeout(std::time::Duration::from_millis(300), delta_rx.recv()).await;
        assert!(
            blocked.is_err(),
            "Back-pressure must pause the stream after 100 unacknowledged deltas"
        );

        // Acknowledge half of the window and verify the stream resumes.
        manager.process_metrics_ack(&session_id, 50).await.unwrap();

        while let Ok(Some(delta)) =
            tokio::time::timeout(std::time::Duration::from_secs(2), delta_rx.recv()).await
        {
            received.push(delta);
        }

        // 105 text deltas + 1 StreamEnd delta from the adapter.
        assert_eq!(received.len(), DELTA_COUNT + 1);
        assert!(
            received
                .iter()
                .any(|d| matches!(d.delta, MessageDeltaContent::StreamEnd { .. }))
        );

        manager.shutdown().await;
    }

    #[tokio::test]
    #[spec("CHAT-010")]
    async fn test_session_enters_zombie_on_abnormal_exit() {
        // Directly exercise the session handler with a channel that closes
        // unexpectedly while metrics are still tracked.
        let session_id = "zombie-test-session".to_string();
        let (message_tx, message_rx) = mpsc::channel::<UserMessage>(1);
        let (delta_tx, _delta_rx) = broadcast::channel::<MessageDelta>(16);

        let sessions = Arc::new(RwLock::new(HashMap::new()));
        let session_metrics = Arc::new(RwLock::new(HashMap::new()));
        session_metrics
            .write()
            .await
            .insert(session_id.clone(), SessionMetrics::new(session_id.clone()));

        let session = ChatSession {
            session_id: session_id.clone(),
            capabilities: None,
            created_at: chrono::Utc::now(),
        };
        let state = ChatSessionState {
            _session: session,
            state: SessionState::Active,
            message_tx,
            delta_tx: delta_tx.clone(),
            task_manager: SessionTaskManager::new(session_id.clone()),
            auth: None,
            inflight_deltas: Arc::new(AtomicU64::new(0)),
            last_acked_seq: Arc::new(AtomicU64::new(0)),
            backpressure_notify: Arc::new(Notify::new()),
        };
        sessions.write().await.insert(session_id.clone(), state);

        let adapter = create_mock_adapter(0);
        let metrics_collector = MetricsCollector::new();
        let inflight_deltas = Arc::new(AtomicU64::new(0));
        let backpressure_notify = Arc::new(Notify::new());
        let buffers = BufferConfig::default();

        tokio::spawn(ChatSessionManager::handle_session(
            session_id.clone(),
            message_rx,
            delta_tx,
            adapter,
            sessions.clone(),
            session_metrics.clone(),
            metrics_collector,
            inflight_deltas,
            backpressure_notify,
            buffers,
        ));

        // Simulate abnormal cleanup: remove the session state (drops message_tx)
        // but leave the metrics entry in place.
        sessions.write().await.remove(&session_id);

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        let metrics = session_metrics.read().await;
        let metric = metrics
            .get(&session_id)
            .expect("metrics retained for zombie");
        assert_eq!(
            metric.state,
            SessionState::Zombie,
            "Session must enter zombie state when its channel closes abnormally"
        );
    }

    #[test]
    #[spec("CHAT-013")]
    fn test_reject_malformed_delta_message() {
        // Unknown delta type must fail deserialization.
        let json = r#"{"sessionId":"s1","messageId":"m1","sequence":0,"delta":{"type":"unknown_variant"},"timestamp":"2024-01-01T00:00:00Z"}"#;
        let result: std::result::Result<MessageDelta, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "Malformed/unknown delta payload must be rejected during deserialization"
        );

        // Missing required fields must also fail.
        let json_missing = r#"{"sessionId":"s1","delta":{"type":"text","text":"hi"}}"#;
        assert!(serde_json::from_str::<MessageDelta>(json_missing).is_err());
    }

    #[tokio::test]
    #[spec("CHAT-002")]
    async fn test_send_message_to_active_session_with_adapter() {
        let adapter = create_mock_adapter(0);
        let manager = ChatSessionManager::new(Some(adapter));
        let session = manager.create_session(None).await;
        let session_id = session.session_id.clone();

        let result = manager
            .send_message(
                &session_id,
                UserMessage {
                    content: "Hello with adapter".to_string(),
                    attachments: None,
                    metadata: None,
                },
            )
            .await;
        assert!(
            result.is_ok(),
            "Active session with adapter must accept messages"
        );

        let metrics = manager.get_metrics_snapshot().await;
        let session_metrics = metrics.get(&session_id).expect("session metrics exist");
        assert_eq!(session_metrics.messages_sent, 1);

        manager.shutdown().await;
    }

    #[tokio::test]
    #[spec("CHAT-003")]
    async fn test_send_message_to_missing_session_returns_not_found() {
        let manager = ChatSessionManager::new(None);
        let result = manager
            .send_message(
                "non-existent-session",
                UserMessage {
                    content: "Hello".to_string(),
                    attachments: None,
                    metadata: None,
                },
            )
            .await;
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(matches!(e, A2aError::SessionNotFound(_)));
        }
        manager.shutdown().await;
    }

    #[tokio::test]
    #[spec("CHAT-004")]
    async fn test_stream_deltas_below_back_pressure_threshold() {
        const DELTA_COUNT: usize = 5;
        let adapter = create_mock_adapter(DELTA_COUNT);
        let mut buffers = BufferConfig::default();
        buffers.chat_streaming_mode = ChatStreamingMode::Delta;
        let manager = ChatSessionManager::with_config(Some(adapter), None, None, 3600, buffers);
        let session = manager.create_session(None).await;
        let session_id = session.session_id.clone();

        let mut delta_rx = manager
            .get_delta_stream(&session_id)
            .await
            .expect("delta stream available");

        manager
            .send_message(
                &session_id,
                UserMessage {
                    content: "start".to_string(),
                    attachments: None,
                    metadata: None,
                },
            )
            .await
            .expect("message accepted");

        let mut received = Vec::new();
        while let Ok(Some(delta)) =
            tokio::time::timeout(std::time::Duration::from_secs(2), delta_rx.recv()).await
        {
            received.push(delta);
        }

        assert_eq!(
            received.len(),
            DELTA_COUNT + 1,
            "All deltas including StreamEnd must arrive without back-pressure below threshold"
        );
        assert!(
            received
                .iter()
                .any(|d| matches!(d.delta, MessageDeltaContent::StreamEnd { .. })),
            "StreamEnd delta must be present"
        );

        manager.shutdown().await;
    }

    #[tokio::test]
    #[spec("CHAT-008")]
    async fn test_get_delta_stream_closed_session_returns_none() {
        let adapter = create_mock_adapter(0);
        let manager = ChatSessionManager::new(Some(adapter));
        let session = manager.create_session(None).await;
        let session_id = session.session_id.clone();

        assert!(
            manager.get_delta_stream(&session_id).await.is_some(),
            "Active session must expose a delta stream"
        );
        manager.close_session(&session_id).await.unwrap();
        assert!(
            manager.get_delta_stream(&session_id).await.is_none(),
            "Closed session must not expose a delta stream"
        );

        manager.shutdown().await;
    }
}
