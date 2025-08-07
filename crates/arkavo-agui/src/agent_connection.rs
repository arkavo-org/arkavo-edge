use crate::streaming::{LatencyTracker, OrderedMessageDelta, StreamOrdering};
use crate::types::{ChatRequest, SubscriptionHandle};
use arkavo_protocol::types::{
    ChatOpenRequest, ChatSession, MessageDelta, MessageDeltaContent, UserMessage,
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
    StateChanged {
        agent_id: String,
        session_id: String,
        patch_count: usize,
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

    async fn connect(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        *self.status.write().await = ConnectionStatus::Connecting;

        let url = format!("ws://{}/ws", self.endpoint);
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
        // Monitor connection health with exponential backoff
        let mut check_interval = Duration::from_secs(5);
        let max_interval = Duration::from_secs(30);
        let mut consecutive_failures = 0;
        const MAX_FAILURES: u32 = 3;

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
                        check_interval = Duration::from_secs(5); // Reset to fast check
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

    /// Send a JSON-RPC request to the agent
    pub async fn send_request(
        &self,
        method: &str,
        params: Value,
        session_id: &str,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let client_guard = self.client.read().await;
        let client = client_guard.as_ref().ok_or("Not connected")?;

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

        let result = client.request(method, rpc_params![params]).await?;

        // Emit telemetry for inbound response
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

        Ok(result)
    }

    /// Subscribe to chat streaming for a given agent with optional authentication
    pub async fn subscribe_to_chat(
        &self,
        agent_id: String,
        ui_tx: mpsc::Sender<crate::types::AgUiEvent>, // Bounded channel
        auth_token: Option<String>,                   // JWT token for authenticated sessions
    ) -> Result<SubscriptionHandle, Box<dyn std::error::Error + Send + Sync>> {
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
                                if pending_acks >= ACK_WINDOW {
                                    if let Some(client) = &*client_for_ack.read().await {
                                        let _ = client.request::<(), _>(
                                            "chat_metrics_ack",
                                            rpc_params![session_id_for_ack.clone(), last_sequence]
                                        ).await;
                                        pending_acks = 0;
                                    }
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
            if pending_acks > 0 {
                if let Some(client) = &*client_for_ack.read().await {
                    let _ = client
                        .request::<(), _>(
                            "chat_metrics_ack",
                            rpc_params![session_id_for_ack.clone(), last_sequence],
                        )
                        .await;
                }
            }

            // Cleanup
            active_subs.write().await.remove(&sub_id_for_task);

            // Remove broadcast channel if no more subscribers
            let mut broadcasts = chat_broadcasts.write().await;
            if let Some(tx) = broadcasts.get(&agent_id_for_task) {
                if tx.receiver_count() == 0 {
                    broadcasts.remove(&agent_id_for_task);
                }
            }
        });

        // Spawn task to forward broadcasts to this specific UI
        let ui_tx_clone = ui_tx.clone();
        let agent_id_for_forward = agent_id.clone();
        tokio::spawn(async move {
            while let Ok(ordered_delta) = broadcast_rx.recv().await {
                let event = crate::types::AgUiEvent::MessageDelta {
                    agent_id: agent_id_for_forward.clone(),
                    message_id: ordered_delta.delta.message_id,
                    delta: match ordered_delta.delta.delta {
                        MessageDeltaContent::Text { text } => {
                            crate::types::MessageDeltaContent::Text { text }
                        }
                        MessageDeltaContent::ToolCall {
                            tool_call_id,
                            name,
                            args_json_fragment,
                            done,
                        } => crate::types::MessageDeltaContent::ToolCall {
                            tool_call_id,
                            name,
                            args_json_fragment,
                            done,
                        },
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

        let _message_id = uuid::Uuid::new_v4().to_string();
        let _trace_id = uuid::Uuid::new_v4().to_string();

        // Get the session ID for this agent
        let session_id = {
            let sessions = self.chat_sessions.read().await;
            sessions
                .get(agent_id)
                .cloned()
                .ok_or("No active chat session for this agent")?
        };

        let client_guard = self.client.read().await;
        let client = client_guard.as_ref().ok_or("Not connected")?;

        // Create user message
        let user_message = UserMessage {
            content: text,
            attachments: None,
            metadata: None,
        };

        // Send the message to the session
        client
            .request::<String, _>("chat_send", rpc_params![session_id, user_message])
            .await
            .map_err(|e| format!("Failed to send message: {e}"))?;

        Ok(())
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
