use crate::streaming::{LatencyTracker, OrderedMessageDelta, StreamOrdering};
use crate::types::{ChatRequest, SubscriptionHandle};
use arkavo_protocol::types::{
    AgentConfigGetRequest, AgentConfigGetResponse, AgentConfigRestoreRequest,
    AgentConfigRestoreResponse, AgentConfigUpdateRequest, AgentConfigUpdateResponse,
    AgentConfigValidateRequest, AgentConfigValidateResponse, ChatOpenRequest, ChatSession,
    MessageDelta, MessageDeltaContent, UserMessage,
};
use jsonrpsee::core::client::{ClientT, SubscriptionClientT};
use jsonrpsee::ws_client::{WsClient, WsClientBuilder};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast, mpsc};
use tokio::time::{Duration, sleep};

/// Represents a persistent connection to an AI agent
#[derive(Clone)]
pub struct AgentConnection {
    agent_id: String,
    endpoint: String,
    client: Arc<RwLock<Option<WsClient>>>,
    status: Arc<RwLock<ConnectionStatus>>,
    telemetry_tx: mpsc::Sender<TelemetryEvent>,
    active_subscriptions: Arc<RwLock<HashMap<String, SubscriptionHandle>>>,
    chat_sessions: Arc<RwLock<HashMap<String, String>>>, // agent_id -> session_id
    chat_broadcasts: Arc<RwLock<HashMap<String, broadcast::Sender<OrderedMessageDelta>>>>,
    context_snapshot: Arc<RwLock<Option<serde_json::Value>>>,
    _stream_ordering: Arc<StreamOrdering>,
    _latency_tracker: Arc<LatencyTracker>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionStatus {
    Connecting,
    Connected,
    Reconnecting { attempt: u32 },
    Failed { reason: String },
}

/// Telemetry events for monitoring agent connections
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TelemetryEvent {
    AgentConnected {
        agent_id: String,
        endpoint: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    AgentDisconnected {
        agent_id: String,
        reason: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    AgentReconnecting {
        agent_id: String,
        attempt: u32,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    MessageRouted {
        agent_id: String,
        session_id: String,
        message_type: String,
        direction: MessageDirection,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    ToolCallExecuted {
        agent_id: String,
        session_id: String,
        tool_name: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    MetricsSnapshot {
        snapshot: arkavo_observability::metrics_snapshot::MetricsSnapshot,
    },
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageDirection {
    Inbound,  // Agent → Gateway
    Outbound, // Gateway → Agent
}

impl AgentConnection {
    pub fn new(
        agent_id: String,
        endpoint: String,
        telemetry_tx: mpsc::Sender<TelemetryEvent>,
    ) -> Self {
        Self {
            agent_id,
            endpoint,
            client: Arc::new(RwLock::new(None)),
            status: Arc::new(RwLock::new(ConnectionStatus::Connecting)),
            telemetry_tx,
            active_subscriptions: Arc::new(RwLock::new(HashMap::new())),
            chat_sessions: Arc::new(RwLock::new(HashMap::new())),
            chat_broadcasts: Arc::new(RwLock::new(HashMap::new())),
            context_snapshot: Arc::new(RwLock::new(None)),
            _stream_ordering: Arc::new(StreamOrdering::new()),
            _latency_tracker: Arc::new(LatencyTracker::new()),
        }
    }

    /// Start the connection with automatic reconnection
    pub async fn start(self: Arc<Self>) {
        let mut reconnect_attempts = 0u32;
        let mut backoff = Duration::from_millis(500);
        let max_backoff = Duration::from_secs(30);

        loop {
            match self.connect().await {
                Ok(()) => {
                    // Reset reconnection state on successful connection
                    reconnect_attempts = 0;
                    backoff = Duration::from_millis(500);

                    // Emit connected telemetry
                    let _ = self
                        .telemetry_tx
                        .send(TelemetryEvent::AgentConnected {
                            agent_id: self.agent_id.clone(),
                            endpoint: self.endpoint.clone(),
                            timestamp: chrono::Utc::now(),
                        })
                        .await;

                    // Wait for disconnection
                    self.wait_for_disconnection().await;

                    // Emit disconnected telemetry
                    let _ = self
                        .telemetry_tx
                        .send(TelemetryEvent::AgentDisconnected {
                            agent_id: self.agent_id.clone(),
                            reason: "Connection lost".to_string(),
                            timestamp: chrono::Utc::now(),
                        })
                        .await;
                }
                Err(e) => {
                    let error_msg = e.to_string();
                    eprintln!(
                        "Failed to connect to agent {}: {}",
                        self.agent_id, error_msg
                    );
                    *self.status.write().await = ConnectionStatus::Failed { reason: error_msg };
                }
            }

            // Reconnection logic
            reconnect_attempts += 1;
            *self.status.write().await = ConnectionStatus::Reconnecting {
                attempt: reconnect_attempts,
            };

            // Emit reconnecting telemetry
            let _ = self
                .telemetry_tx
                .send(TelemetryEvent::AgentReconnecting {
                    agent_id: self.agent_id.clone(),
                    attempt: reconnect_attempts,
                    timestamp: chrono::Utc::now(),
                })
                .await;

            println!(
                "Reconnecting to agent {} in {:?} (attempt {})",
                self.agent_id, backoff, reconnect_attempts
            );

            sleep(backoff).await;

            // Exponential backoff with jitter
            backoff = std::cmp::min(backoff * 2, max_backoff);
            let jitter = Duration::from_millis(rand::random::<u64>() % 1000);
            backoff += jitter;
        }
    }

    pub async fn connect(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        *self.status.write().await = ConnectionStatus::Connecting;

        let url = format!("ws://{}", self.endpoint);
        println!("Connecting to agent {} at {}", self.agent_id, url);

        let client = WsClientBuilder::default()
            .request_timeout(Duration::from_secs(10))
            .connection_timeout(Duration::from_secs(10))
            .max_concurrent_requests(100)
            .build(&url)
            .await?;

        *self.client.write().await = Some(client);
        *self.status.write().await = ConnectionStatus::Connected;

        Ok(())
    }

    async fn wait_for_disconnection(&self) {
        // Monitor connection health — detect dead connections quickly
        let mut check_interval = Duration::from_secs(2);
        let max_interval = Duration::from_secs(15);
        let mut consecutive_failures = 0;
        const MAX_FAILURES: u32 = 2;

        loop {
            sleep(check_interval).await;

            // Check if client is still healthy
            let is_healthy = if let Some(client) = &*self.client.read().await {
                // Try a simple ping or method call to check connection
                match client
                    .request::<Value, _>("rpc.discover", rpc_params![])
                    .await
                {
                    Ok(_) => {
                        consecutive_failures = 0;
                        check_interval = Duration::from_secs(2); // Reset to fast check
                        true
                    }
                    Err(e) => {
                        consecutive_failures += 1;
                        eprintln!(
                            "Agent {} health check failed ({}): {}",
                            self.agent_id, consecutive_failures, e
                        );
                        false
                    }
                }
            } else {
                // No client
                false
            };

            if !is_healthy && consecutive_failures >= MAX_FAILURES {
                // Connection definitively lost
                eprintln!(
                    "Agent {} connection lost after {} failures",
                    self.agent_id, consecutive_failures
                );

                // Clear the dead client so callers don't use a stale connection
                *self.client.write().await = None;

                // Send terminal deltas to all active chat broadcasts
                self.notify_connection_lost().await;
                break;
            }

            // Exponential backoff for next check
            if !is_healthy {
                check_interval = std::cmp::min(check_interval * 2, max_interval);
            }
        }
    }

    async fn notify_connection_lost(&self) {
        // Clear stale session IDs so reconnection creates fresh sessions
        self.chat_sessions.write().await.clear();

        let broadcasts = self.chat_broadcasts.read().await;

        for (agent_id, broadcast_tx) in broadcasts.iter() {
            // Create a terminal delta indicating connection lost
            let terminal_delta = OrderedMessageDelta {
                delta: MessageDelta {
                    session_id: String::new(),
                    message_id: uuid::Uuid::new_v4().to_string(),
                    sequence: 0,
                    delta: MessageDeltaContent::Text {
                        text: "\n\n[Connection to agent lost]".to_string(),
                    },
                    timestamp: chrono::Utc::now(),
                },
                sequence: u64::MAX, // Special sequence for terminal messages
                trace_id: uuid::Uuid::new_v4().to_string(),
                agent_id: agent_id.clone(),
            };

            // Send to all subscribers
            let _ = broadcast_tx.send(terminal_delta);
        }

        // No local LLM streams to abort - all handled by agent's protocol server
    }

    pub async fn get_status(&self) -> ConnectionStatus {
        self.status.read().await.clone()
    }

    pub async fn is_connected(&self) -> bool {
        self.client.read().await.is_some()
    }

    /// Send a JSON-RPC request to the agent, retrying once after reconnect on connection errors
    pub async fn send_request(
        &self,
        method: &str,
        params: Value,
        session_id: &str,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        // Emit telemetry for outbound message
        let _ = self
            .telemetry_tx
            .send(TelemetryEvent::MessageRouted {
                agent_id: self.agent_id.clone(),
                session_id: session_id.to_string(),
                message_type: method.to_string(),
                direction: MessageDirection::Outbound,
                timestamp: chrono::Utc::now(),
            })
            .await;

        // First attempt
        let first_err = {
            let client_guard = self.client.read().await;
            let client = match client_guard.as_ref() {
                Some(c) => c,
                None => return Err("Not connected".into()),
            };
            match client.request(method, rpc_params![params.clone()]).await {
                Ok(result) => {
                    self.emit_response_telemetry(method, session_id).await;
                    return Ok(result);
                }
                Err(e) => e,
            }
        }; // read lock dropped

        // Only retry on connection-level errors (not application errors like invalid params)
        if !is_connection_error(&first_err) {
            return Err(first_err.into());
        }

        eprintln!(
            "Agent {} request {method} failed with connection error, reconnecting: {first_err}",
            self.agent_id
        );

        // Reconnect and retry once
        if let Err(e) = self.connect().await {
            return Err(format!(
                "Reconnect failed after connection error: {e} (original: {first_err})"
            )
            .into());
        }

        let client_guard = self.client.read().await;
        let client = client_guard
            .as_ref()
            .ok_or("Not connected after reconnect")?;
        let result = client.request(method, rpc_params![params]).await?;
        self.emit_response_telemetry(method, session_id).await;
        Ok(result)
    }

    async fn emit_response_telemetry(&self, method: &str, session_id: &str) {
        let _ = self
            .telemetry_tx
            .send(TelemetryEvent::MessageRouted {
                agent_id: self.agent_id.clone(),
                session_id: session_id.to_string(),
                message_type: format!("{method}_response"),
                direction: MessageDirection::Inbound,
                timestamp: chrono::Utc::now(),
            })
            .await;
    }

    /// Subscribe to chat streaming for a given agent with optional authentication
    pub async fn subscribe_to_chat(
        &self,
        agent_id: String,
        ui_tx: mpsc::Sender<crate::types::AgUiEvent>, // Bounded channel
        auth_token: Option<String>,                   // JWT token for authenticated sessions
    ) -> Result<SubscriptionHandle, Box<dyn std::error::Error + Send + Sync>> {
        // Wait for connection if agent is reconnecting (up to 10s)
        let mut retries = 0;
        loop {
            let status = self.status.read().await.clone();
            match status {
                ConnectionStatus::Connected => break,
                ConnectionStatus::Connecting | ConnectionStatus::Reconnecting { .. }
                    if retries < 10 =>
                {
                    retries += 1;
                    drop(status);
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }
                _ => {
                    return Err(format!(
                        "Agent {} not connected (status: {:?})",
                        self.agent_id, status
                    )
                    .into());
                }
            }
        }

        let client_guard = self.client.read().await;
        let client = client_guard.as_ref().ok_or("Not connected")?;

        // Ensure we're connecting to the right agent
        if agent_id != self.agent_id {
            return Err("Agent ID mismatch".into());
        }

        // Create or get broadcast channel for this agent
        let broadcast_tx = {
            let mut broadcasts = self.chat_broadcasts.write().await;
            broadcasts
                .entry(agent_id.clone())
                .or_insert_with(|| {
                    let (tx, _) = broadcast::channel(32); // Buffer size for late joiners
                    tx
                })
                .clone()
        };

        // Subscribe to this broadcast for this UI connection
        let mut broadcast_rx = broadcast_tx.subscribe();

        // Create subscription handle
        let sub_id = uuid::Uuid::new_v4().to_string();
        let (sub_handle, cancel_rx) = SubscriptionHandle::new(sub_id.clone());

        // We'll store the subscription handle after setting up the task

        // Clone necessary data for the subscription task
        let agent_id_for_task = agent_id.clone();
        let telemetry_tx = self.telemetry_tx.clone();
        let active_subs = self.active_subscriptions.clone();
        let chat_broadcasts = self.chat_broadcasts.clone();
        let sub_id_for_task = sub_id.clone();

        // Create chat request with session ID
        let _request = ChatRequest {
            message: String::new(), // Initial subscription
            context: None,
            session_id: Some(sub_id.clone()),
        };

        // Open a new chat session with optional authentication
        let open_request = ChatOpenRequest {
            token: auth_token.clone(),
            context: None,
            metadata: None,
        };

        let session: ChatSession = client
            .request("chat_open", rpc_params![open_request])
            .await
            .map_err(|e| format!("Failed to open chat session: {e}"))?;

        // Store the session ID
        self.chat_sessions
            .write()
            .await
            .insert(agent_id.clone(), session.session_id.clone());

        // Start the subscription to the agent's session
        let mut subscription = client
            .subscribe::<MessageDelta, _>(
                "chat_stream",
                rpc_params![session.session_id.clone()],
                "chat_stream_unsubscribe",
            )
            .await?;

        // Spawn task to handle streaming with back-pressure management
        let session_id_for_ack = session.session_id.clone();
        let client_for_ack = self.client.clone();

        tokio::spawn(async move {
            let mut cancel_rx = cancel_rx;
            let mut last_sequence = 0u64;
            let mut pending_acks = 0u64;
            const ACK_WINDOW: u64 = 50; // Send ack every 50 messages

            loop {
                tokio::select! {
                    // Handle incoming deltas from agent
                    delta_result = subscription.next() => {
                        match delta_result {
                            Some(Ok(delta)) => {
                                // Extract sequence number from delta
                                let sequence = delta.sequence;
                                last_sequence = sequence;
                                pending_acks += 1;

                                // Convert to ordered delta with proper sequence tracking
                                let ordered_delta = OrderedMessageDelta {
                                    delta,
                                    sequence,
                                    trace_id: uuid::Uuid::new_v4().to_string(),
                                    agent_id: agent_id_for_task.clone(),
                                };

                                // Broadcast to all UIs watching this session
                                let _ = broadcast_tx.send(ordered_delta);

                                // Send MetricsAck for back-pressure management
                                if pending_acks >= ACK_WINDOW
                                    && let Some(client) = &*client_for_ack.read().await {
                                        let _ = client.request::<(), _>(
                                            "chat_metrics_ack",
                                            rpc_params![session_id_for_ack.clone(), last_sequence]
                                        ).await;
                                        pending_acks = 0;
                                    }

                                // Send telemetry
                                let _ = telemetry_tx
                                    .send(TelemetryEvent::MessageRouted {
                                        agent_id: agent_id_for_task.clone(),
                                        session_id: sub_id_for_task.clone(),
                                        message_type: "message_delta".to_string(),
                                        direction: MessageDirection::Inbound,
                                        timestamp: chrono::Utc::now(),
                                    })
                                    .await;
                            }
                            Some(Err(e)) => {
                                eprintln!("Subscription error: {e}");
                                break;
                            }
                            None => {
                                // Subscription ended
                                break;
                            }
                        }
                    }
                    // Handle cancellation
                    _ = &mut cancel_rx => {
                        // Unsubscribe from agent
                        let _ = subscription.unsubscribe().await;
                        break;
                    }
                }
            }

            // Send final MetricsAck if we have pending acks
            if pending_acks > 0
                && let Some(client) = &*client_for_ack.read().await
            {
                let _ = client
                    .request::<(), _>(
                        "chat_metrics_ack",
                        rpc_params![session_id_for_ack.clone(), last_sequence],
                    )
                    .await;
            }

            // Cleanup
            active_subs.write().await.remove(&sub_id_for_task);

            // Remove broadcast channel if no more subscribers
            let mut broadcasts = chat_broadcasts.write().await;
            if let Some(tx) = broadcasts.get(&agent_id_for_task)
                && tx.receiver_count() == 0
            {
                broadcasts.remove(&agent_id_for_task);
            }
        });

        // Spawn task to forward broadcasts to this specific UI
        let ui_tx_clone = ui_tx.clone();
        let agent_id_for_forward = agent_id.clone();
        let context_snapshot_store = self.context_snapshot.clone();
        tokio::spawn(async move {
            while let Ok(ordered_delta) = broadcast_rx.recv().await {
                // Intercept Metadata deltas — convert to UI events instead of forwarding
                if let MessageDeltaContent::Metadata { ref key, ref value } =
                    ordered_delta.delta.delta
                {
                    if key == "model_selected" {
                        let model_event = crate::types::AgUiEvent::ModelSelected {
                            agent_id: agent_id_for_forward.clone(),
                            provider: value["category"].as_str().unwrap_or("local").to_string(),
                            model: value["model"].as_str().unwrap_or("unknown").to_string(),
                            estimated_cost: arkavo_budget::TokenCost::from_dollars(
                                value["estimated_cost_usd"].as_f64().unwrap_or(0.0),
                            ),
                            reason: value["reasoning"].as_str().unwrap_or("").to_string(),
                            event_id: uuid::Uuid::new_v4().to_string(),
                        };
                        let _ = ui_tx_clone.try_send(model_event);
                    }
                    let telemetry = crate::types::AgUiEvent::TelemetryEvent {
                        event_type: key.clone(),
                        agent_id: agent_id_for_forward.clone(),
                        details: value.clone(),
                        timestamp: chrono::Utc::now().to_rfc3339(),
                    };
                    let _ = ui_tx_clone.try_send(telemetry);

                    // Forward teaching_intent as MessageDelta so chat UI can render badges
                    if key == "teaching_intent" {
                        let teaching_event = crate::types::AgUiEvent::MessageDelta {
                            agent_id: agent_id_for_forward.clone(),
                            message_id: ordered_delta.delta.message_id.clone(),
                            delta: crate::types::MessageDeltaContent::Metadata {
                                key: key.clone(),
                                value: value.clone(),
                            },
                        };
                        let _ = ui_tx_clone.try_send(teaching_event);
                    }
                    continue; // Don't forward other Metadata as MessageDelta
                }

                let event = crate::types::AgUiEvent::MessageDelta {
                    agent_id: agent_id_for_forward.clone(),
                    message_id: ordered_delta.delta.message_id,
                    delta: match ordered_delta.delta.delta {
                        MessageDeltaContent::Text { ref text } => {
                            // Detect @context introspection response
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(text)
                                && parsed.get("messages").is_some()
                                && parsed.get("cycle").is_some()
                            {
                                *context_snapshot_store.write().await = Some(parsed);
                            }
                            crate::types::MessageDeltaContent::Text { text: text.clone() }
                        }
                        MessageDeltaContent::ToolCall {
                            tool_call_id,
                            name,
                            args_json_fragment,
                            done,
                        } => {
                            if done && let Some(ref tool_name) = name {
                                let tool_telemetry = crate::types::AgUiEvent::TelemetryEvent {
                                    event_type: "tool_call_executed".to_string(),
                                    agent_id: agent_id_for_forward.clone(),
                                    details: serde_json::json!({
                                        "tool_call_id": tool_call_id,
                                        "tool_name": tool_name,
                                    }),
                                    timestamp: chrono::Utc::now().to_rfc3339(),
                                };
                                let _ = ui_tx_clone.try_send(tool_telemetry);
                            }
                            crate::types::MessageDeltaContent::ToolCall {
                                tool_call_id,
                                name,
                                args_json_fragment,
                                done,
                            }
                        }
                        MessageDeltaContent::StreamEnd { .. } => {
                            // Convert stream end to text for UI
                            crate::types::MessageDeltaContent::Text {
                                text: "[Stream ended]".to_string(),
                            }
                        }
                        MessageDeltaContent::Error { code, message } => {
                            // Convert error to text for UI
                            crate::types::MessageDeltaContent::Text {
                                text: format!("[Error {code}] {message}"),
                            }
                        }
                        MessageDeltaContent::ToolResult {
                            tool_call_id,
                            content,
                            is_error,
                        } => crate::types::MessageDeltaContent::ToolResult {
                            tool_call_id,
                            content,
                            is_error,
                        },
                        MessageDeltaContent::Metadata { .. } => continue,
                    },
                };

                // Use try_send for back-pressure handling
                if ui_tx_clone.try_send(event).is_err() {
                    // Channel full or disconnected
                    break;
                }
            }
        });

        // Store the subscription handle and return it
        {
            let mut subs = self.active_subscriptions.write().await;
            subs.insert(sub_id.clone(), sub_handle);
        }

        // Return a new handle with just the ID for the caller
        Ok(SubscriptionHandle {
            id: sub_id,
            cancel_tx: None,
        })
    }

    /// Send a user message within an existing chat subscription
    pub async fn send_user_message(
        &self,
        agent_id: &str,
        text: String,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Ensure we're sending to the right agent
        if agent_id != self.agent_id {
            return Err("Agent ID mismatch".into());
        }

        // Get the broadcast channel for this agent (not used in new protocol but kept for consistency)
        let _broadcast_tx = {
            let broadcasts = self.chat_broadcasts.read().await;
            broadcasts
                .get(agent_id)
                .ok_or("No active chat subscription for this agent")?
                .clone()
        };

        // Get the session ID for this agent
        let session_id = {
            let sessions = self.chat_sessions.read().await;
            sessions
                .get(agent_id)
                .cloned()
                .ok_or("No active chat session for this agent")?
        };

        let user_message = UserMessage {
            content: text.clone(),
            attachments: None,
            metadata: None,
        };

        // Try sending with current session
        let client_guard = self.client.read().await;
        let client = client_guard.as_ref().ok_or("Not connected")?;

        let result = client
            .request::<(), _>("chat_send", rpc_params![session_id, user_message])
            .await;

        match result {
            Ok(()) => Ok(()),
            Err(e) if e.to_string().contains("Session not found") => {
                drop(client_guard); // release read lock before reconnect

                // Session is stale — open a fresh one and retry
                tracing::info!(agent_id, "Stale chat session detected, opening new session");
                let new_session_id = self.reopen_chat_session(agent_id).await?;

                let client_guard = self.client.read().await;
                let client = client_guard.as_ref().ok_or("Not connected")?;
                let retry_msg = UserMessage {
                    content: text,
                    attachments: None,
                    metadata: None,
                };
                client
                    .request::<(), _>("chat_send", rpc_params![new_session_id, retry_msg])
                    .await
                    .map_err(|e| format!("Failed to send message after session refresh: {e}"))?;
                Ok(())
            }
            Err(e) => Err(format!("Failed to send message: {e}").into()),
        }
    }

    /// Return the latest stored @context introspection snapshot
    pub async fn get_latest_context_snapshot(&self) -> Option<serde_json::Value> {
        self.context_snapshot.read().await.clone()
    }

    /// Send an introspection message (like @context) that auto-opens a session if needed.
    /// Unlike send_user_message, does not require an active chat subscription.
    pub async fn send_introspection(
        &self,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Get or create a chat session
        let session_id = {
            let sessions = self.chat_sessions.read().await;
            sessions.get(&self.agent_id).cloned()
        };
        let session_id = match session_id {
            Some(id) => id,
            None => self.reopen_chat_session(&self.agent_id).await?,
        };

        let user_message = UserMessage {
            content: text.to_string(),
            attachments: None,
            metadata: None,
        };

        let client_guard = self.client.read().await;
        let client = client_guard.as_ref().ok_or("Not connected")?;

        match client
            .request::<(), _>("chat_send", rpc_params![session_id, user_message])
            .await
        {
            Ok(()) => Ok(()),
            Err(e) if e.to_string().contains("Session not found") => {
                drop(client_guard);
                let new_session_id = self.reopen_chat_session(&self.agent_id).await?;
                let client_guard = self.client.read().await;
                let client = client_guard.as_ref().ok_or("Not connected")?;
                let retry_msg = UserMessage {
                    content: text.to_string(),
                    attachments: None,
                    metadata: None,
                };
                client
                    .request::<(), _>("chat_send", rpc_params![new_session_id, retry_msg])
                    .await
                    .map_err(|e| format!("Introspection send failed: {e}"))?;
                Ok(())
            }
            Err(e) => Err(format!("Introspection send failed: {e}").into()),
        }
    }

    /// Re-open a chat session for an agent after the previous one became stale
    async fn reopen_chat_session(
        &self,
        agent_id: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let client_guard = self.client.read().await;
        let client = client_guard.as_ref().ok_or("Not connected")?;

        let open_request = ChatOpenRequest {
            token: None,
            context: None,
            metadata: None,
        };

        let session: ChatSession = client
            .request("chat_open", rpc_params![open_request])
            .await
            .map_err(|e| format!("Failed to reopen chat session: {e}"))?;

        self.chat_sessions
            .write()
            .await
            .insert(agent_id.to_string(), session.session_id.clone());

        tracing::info!(
            agent_id,
            session_id = %session.session_id,
            "Reopened chat session after stale session"
        );

        Ok(session.session_id)
    }

    /// Unsubscribe from chat for a specific agent
    pub async fn unsubscribe_chat(
        &self,
        agent_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Remove the broadcast channel
        self.chat_broadcasts.write().await.remove(agent_id);

        // No local LLM streams to cancel - all handled by agent's protocol server

        Ok(())
    }

    /// Cancel all active subscriptions
    pub async fn cancel_all_subscriptions(&self) {
        let mut subs = self.active_subscriptions.write().await;
        for (_, mut handle) in subs.drain() {
            handle.cancel();
        }

        // Clear all broadcast channels
        self.chat_broadcasts.write().await.clear();

        // No local LLM streams to abort - all handled by agent's protocol server
    }

    /// Get compute budget status via JSON-RPC telemetry
    pub async fn get_compute_budget(
        &self,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        let client_guard = self.client.read().await;
        let client = client_guard.as_ref().ok_or("Not connected to agent")?;

        let result: serde_json::Value = client
            .request("budget.compute_status", rpc_params![])
            .await?;

        Ok(result)
    }

    /// Get per-agent process metrics (RSS, CPU) via JSON-RPC
    pub async fn get_system_metrics(
        &self,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        let client_guard = self.client.read().await;
        let client = client_guard.as_ref().ok_or("Not connected to agent")?;

        let result: serde_json::Value = client.request("system.metrics", rpc_params![]).await?;

        Ok(result)
    }

    /// Aggregate snapshot of every subject this agent's MCP-T trust service
    /// scores: composite + dimensions per subject, plus the most recent
    /// `behavior.trace` events across all subjects. The browser's "Published
    /// Trust" panel consumes one of these per refresh.
    ///
    /// Wire path: `trust.subjects` (private adjunct, returns the list) →
    /// per-subject `trust/query` and `trust/history` (both spec methods),
    /// fanned out concurrently. The whole call rides the existing JSON-RPC
    /// WebSocket — no new transport.
    pub async fn get_published_trust(
        &self,
    ) -> Result<crate::types::PublishedTrustSnapshot, Box<dyn std::error::Error + Send + Sync>>
    {
        use crate::types::{
            BehaviorTraceView, DimensionView, PublishedTrustSnapshot, TrustScoreView,
        };

        let client_guard = self.client.read().await;
        let client = client_guard.as_ref().ok_or("Not connected to agent")?;

        let subjects_resp: serde_json::Value =
            client.request("trust.subjects", rpc_params![]).await?;
        let subjects: Vec<String> = subjects_resp
            .get("subjects")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        // Concurrent fan-out — N subjects × (1 query + 1 history) round trips,
        // executed in parallel so the panel refresh stays under one second
        // even with a swarm of peers.
        let queries = subjects.iter().map(|s| {
            let s = s.clone();
            async move {
                let q = client
                    .request::<serde_json::Value, _>(
                        "trust/query",
                        rpc_params![serde_json::json!({ "subject_id": s })],
                    )
                    .await
                    .ok();
                let h = client
                    .request::<serde_json::Value, _>(
                        "trust/history",
                        rpc_params![serde_json::json!({
                            "subject_id": s,
                            "event_types": ["behavior.trace"],
                            "limit": 5,
                        })],
                    )
                    .await
                    .ok();
                (s, q, h)
            }
        });
        let results = futures::future::join_all(queries).await;

        let mut self_score: Option<TrustScoreView> = None;
        let mut peers: Vec<TrustScoreView> = Vec::new();
        let mut recent_traces: Vec<BehaviorTraceView> = Vec::new();
        let mut provider_did = String::new();

        let self_subject_id = self.agent_id.clone();

        for (subject_id, query, history) in results {
            if let Some(q) = query.as_ref()
                && let Some(score) = q.get("trust_score")
            {
                if provider_did.is_empty()
                    && let Some(p) = score.get("provider_id").and_then(|v| v.as_str())
                {
                    provider_did = p.to_string();
                }
                let view = TrustScoreView {
                    subject_id: subject_id.clone(),
                    composite: score
                        .get("score")
                        .and_then(|s| s.get("composite"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32,
                    dimensions: score
                        .get("score")
                        .and_then(|s| s.get("dimensions"))
                        .and_then(|d| d.as_object())
                        .map(|map| {
                            let mut v: Vec<DimensionView> = map
                                .iter()
                                .map(|(name, val)| DimensionView {
                                    name: name.clone(),
                                    value: val.get("value").and_then(|x| x.as_u64()).unwrap_or(0)
                                        as u32,
                                    confidence: val
                                        .get("confidence")
                                        .and_then(|x| x.as_f64())
                                        .unwrap_or(0.0),
                                    evidence_count: val
                                        .get("evidence_count")
                                        .and_then(|x| x.as_u64())
                                        .unwrap_or(0)
                                        as u32,
                                })
                                .collect();
                            v.sort_by(|a, b| a.name.cmp(&b.name));
                            v
                        })
                        .unwrap_or_default(),
                    validity_expires_at: score
                        .get("validity")
                        .and_then(|v| v.get("expires_at"))
                        .and_then(|v| v.as_str())
                        .map(String::from),
                };
                if subject_id == self_subject_id {
                    self_score = Some(view);
                } else {
                    peers.push(view);
                }
            }

            if let Some(h) = history.as_ref()
                && let Some(events) = h.get("events").and_then(|v| v.as_array())
            {
                for ev in events {
                    let payload = ev.get("payload");
                    let trace_view = BehaviorTraceView {
                        trace_id: payload
                            .and_then(|p| p.get("trace_id"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        contract_id: payload
                            .and_then(|p| p.get("contract_id"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        subject_id: subject_id.clone(),
                        timestamp: ev
                            .get("timestamp")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        fidelity_ratio: payload
                            .and_then(|p| p.get("fidelity_ratio"))
                            .and_then(|v| v.as_f64())
                            .unwrap_or(1.0),
                        total_tool_calls: payload
                            .and_then(|p| p.get("total_tool_calls"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32,
                        undeclared_tool_calls: payload
                            .and_then(|p| p.get("undeclared_tool_calls"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32,
                    };
                    recent_traces.push(trace_view);
                }
            }
        }

        peers.sort_by_key(|p| std::cmp::Reverse(p.composite));
        recent_traces.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        recent_traces.truncate(5);

        Ok(PublishedTrustSnapshot {
            provider_did,
            self_subject_id,
            self_score,
            peers,
            recent_traces,
            fetched_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    /// Subscribe to push-based metrics stream from agent.
    /// Replaces polling of system.metrics + budget.compute_status.
    pub async fn subscribe_metrics(
        &self,
        tx: mpsc::Sender<crate::types::AgUiEvent>,
        security_handler: Arc<RwLock<crate::security_handler::SecurityHandler>>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let client_guard = self.client.read().await;
        let client = client_guard.as_ref().ok_or("Not connected to agent")?;

        let mut subscription = client
            .subscribe::<serde_json::Value, _>(
                "system.metrics.subscribe",
                rpc_params![],
                "system.metrics.unsubscribe",
            )
            .await?;
        drop(client_guard);

        let agent_id = self.agent_id.clone();
        tokio::spawn(async move {
            while let Some(Ok(metrics)) = subscription.next().await {
                // Forward system metrics
                let event = crate::types::AgUiEvent::AgentSystemMetrics {
                    agent_id: agent_id.clone(),
                    rss_mb: metrics["rss_mb"].as_f64().unwrap_or(0.0),
                    cpu_percent: metrics["cpu_percent"].as_f64().unwrap_or(0.0),
                    pid: metrics["pid"].as_u64().unwrap_or(0) as u32,
                    total_ram_mb: metrics["total_ram_mb"].as_f64(),
                    available_ram_mb: metrics["available_ram_mb"].as_f64(),
                };
                let _ = tx.send(event).await;

                // Forward compute budget
                if let Some(budget) = metrics.get("compute_budget") {
                    let event = crate::types::AgUiEvent::ComputeBudgetUpdate {
                        agent_id: agent_id.clone(),
                        compute_budget: budget.clone(),
                        timestamp: chrono::Utc::now().to_rfc3339(),
                    };
                    let _ = tx.send(event).await;
                }

                // Update Iroh status
                if let Some(iroh) = metrics.get("iroh_active").and_then(|v| v.as_bool()) {
                    let sec = security_handler.read().await;
                    sec.update_agent_iroh(&agent_id, iroh).await;
                    drop(sec);
                }

                // Forward context topology as telemetry event
                if let Some(ctx) = metrics.get("context_topology") {
                    let event = crate::types::AgUiEvent::TelemetryEvent {
                        event_type: "context_topology".to_string(),
                        agent_id: agent_id.clone(),
                        details: ctx.clone(),
                        timestamp: chrono::Utc::now().to_rfc3339(),
                    };
                    let _ = tx.send(event).await;
                }

                // Forward subsystem timing
                if let Some(timing) = metrics.get("subsystem_timing") {
                    let registry = arkavo_observability::subsystem_timing::global_timing();
                    if let Some(ms) = timing.get("routerDecisionAvgMs").and_then(|v| v.as_f64())
                        && ms > 0.0
                    {
                        registry.router_decisions.record(ms as u64);
                    }
                    if let Some(ms) = timing
                        .get("conductorOrchestrationAvgMs")
                        .and_then(|v| v.as_f64())
                        && ms > 0.0
                    {
                        registry.conductor_orchestration.record(ms as u64);
                    }
                    if let Some(ms) = timing.get("mcpToolAvgMs").and_then(|v| v.as_f64())
                        && ms > 0.0
                    {
                        registry.mcp_tools.record(ms as u64);
                    }
                    if let Some(ms) = timing.get("inferenceAvgMs").and_then(|v| v.as_f64())
                        && ms > 0.0
                    {
                        registry.inference.record(ms as u64);
                    }
                }
            }
        });

        Ok(())
    }

    /// Get KAS public key from agent (returns None if KAS not enabled)
    pub async fn get_kas_public_key(
        &self,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        let client_guard = self.client.read().await;
        let client = client_guard.as_ref().ok_or("Not connected to agent")?;

        let result: serde_json::Value = client
            .request(
                "kas.publicKey",
                rpc_params![serde_json::json!({"request": {}})],
            )
            .await?;

        Ok(result)
    }

    /// Get agent configuration
    pub async fn get_config(
        &self,
        include_backups: bool,
    ) -> Result<AgentConfigGetResponse, Box<dyn std::error::Error + Send + Sync>> {
        let client_guard = self.client.read().await;
        let client = client_guard.as_ref().ok_or("Not connected to agent")?;

        let request = AgentConfigGetRequest {
            agent_id: self.agent_id.clone(),
            include_backups,
        };

        let response = client
            .request::<AgentConfigGetResponse, _>(
                "agent.config.get",
                vec![serde_json::to_value(request)?],
            )
            .await?;

        Ok(response)
    }

    /// Update agent configuration
    pub async fn update_config(
        &self,
        content: String,
        expected_version: Option<String>,
        create_backup: bool,
    ) -> Result<AgentConfigUpdateResponse, Box<dyn std::error::Error + Send + Sync>> {
        let client_guard = self.client.read().await;
        let client = client_guard.as_ref().ok_or("Not connected to agent")?;

        let request = AgentConfigUpdateRequest {
            agent_id: self.agent_id.clone(),
            content,
            expected_version,
            create_backup,
        };

        let response = client
            .request::<AgentConfigUpdateResponse, _>(
                "agent.config.update",
                vec![serde_json::to_value(request)?],
            )
            .await?;

        Ok(response)
    }

    /// Validate agent configuration
    pub async fn validate_config(
        &self,
        content: String,
    ) -> Result<AgentConfigValidateResponse, Box<dyn std::error::Error + Send + Sync>> {
        let client_guard = self.client.read().await;
        let client = client_guard.as_ref().ok_or("Not connected to agent")?;

        let request = AgentConfigValidateRequest {
            agent_id: self.agent_id.clone(),
            content,
        };

        let response = client
            .request::<AgentConfigValidateResponse, _>(
                "agent.config.validate",
                vec![serde_json::to_value(request)?],
            )
            .await?;

        Ok(response)
    }

    /// Restore agent configuration from backup
    pub async fn restore_config(
        &self,
        backup_filename: String,
    ) -> Result<AgentConfigRestoreResponse, Box<dyn std::error::Error + Send + Sync>> {
        let client_guard = self.client.read().await;
        let client = client_guard.as_ref().ok_or("Not connected to agent")?;

        let request = AgentConfigRestoreRequest {
            agent_id: self.agent_id.clone(),
            backup_filename,
        };

        let response = client
            .request::<AgentConfigRestoreResponse, _>(
                "agent.config.restore",
                vec![serde_json::to_value(request)?],
            )
            .await?;

        Ok(response)
    }
}

/// Check if a jsonrpsee error indicates the connection is dead (vs an application-level error)
fn is_connection_error(err: &jsonrpsee::core::ClientError) -> bool {
    use jsonrpsee::core::ClientError;
    matches!(
        err,
        ClientError::Transport(_) | ClientError::RestartNeeded(_) | ClientError::RequestTimeout
    )
}

impl Drop for AgentConnection {
    fn drop(&mut self) {
        // Cancel all subscriptions when the connection is dropped
        let subs = self.active_subscriptions.clone();
        let broadcasts = self.chat_broadcasts.clone();

        tokio::spawn(async move {
            let mut subs_guard = subs.write().await;
            for (_, mut handle) in subs_guard.drain() {
                handle.cancel();
            }

            broadcasts.write().await.clear();
        });
    }
}

// Re-export for convenience
pub use jsonrpsee::rpc_params;
