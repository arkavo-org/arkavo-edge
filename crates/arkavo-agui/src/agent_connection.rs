use crate::types::{ChatRequest, SubscriptionHandle};
use arkavo_protocol::types::{MessageDelta, MessageDeltaContent};
use jsonrpsee::core::client::{ClientT, SubscriptionClientT};
use jsonrpsee::ws_client::{WsClient, WsClientBuilder};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, broadcast};
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
    chat_broadcasts: Arc<RwLock<HashMap<String, broadcast::Sender<MessageDelta>>>>,
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
            chat_broadcasts: Arc::new(RwLock::new(HashMap::new())),
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
        // In a real implementation, this would monitor the connection health
        // For now, we'll just wait indefinitely
        loop {
            sleep(Duration::from_secs(30)).await;

            // Check if client is still healthy
            if let Some(client) = &*self.client.read().await {
                // Try a simple ping or method call to check connection
                match client
                    .request::<Value, _>("rpc.discover", rpc_params![])
                    .await
                {
                    Ok(_) => continue, // Still connected
                    Err(_) => break,   // Connection lost
                }
            } else {
                break;
            }
        }
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
                message_type: format!("{}_response", method),
                direction: MessageDirection::Inbound,
                timestamp: chrono::Utc::now(),
            })
            .await;

        Ok(result)
    }

    /// Subscribe to chat streaming for a given agent
    pub async fn subscribe_to_chat(
        &self,
        agent_id: String,
        ui_tx: mpsc::Sender<crate::types::AgUiEvent>, // Bounded channel
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
        let request = ChatRequest {
            message: String::new(), // Initial subscription
            context: None,
            session_id: Some(sub_id.clone()),
        };
        
        // Start the subscription to the agent
        let mut subscription = client
            .subscribe::<MessageDelta, _>(
                "chat_subscribe",
                rpc_params![request],
                "chat_unsubscribe",
            )
            .await?;

        // Spawn task to handle streaming
        tokio::spawn(async move {
            let mut cancel_rx = cancel_rx;

            loop {
                tokio::select! {
                    // Handle incoming deltas from agent
                    delta_result = subscription.next() => {
                        match delta_result {
                            Some(Ok(delta)) => {
                                // Broadcast to all UIs watching this session
                                let _ = broadcast_tx.send(delta);

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
                                eprintln!("Subscription error: {}", e);
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
            while let Ok(delta) = broadcast_rx.recv().await {
                let event = crate::types::AgUiEvent::MessageDelta {
                    agent_id: agent_id_for_forward.clone(),
                    message_id: delta.message_id,
                    delta: match delta.delta {
                        MessageDeltaContent::Text { text } => {
                            crate::types::MessageDeltaContent::Text { text }
                        }
                        MessageDeltaContent::ToolCall { tool_call_id, delta } => {
                            crate::types::MessageDeltaContent::ToolCall {
                                tool_call_id,
                                delta,
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
        
        // Get the broadcast channel for this agent
        let broadcast_tx = {
            let broadcasts = self.chat_broadcasts.read().await;
            broadcasts
                .get(agent_id)
                .ok_or("No active chat subscription for this agent")?
                .clone()
        };
        
        // For now, we'll create a simple echo response
        // In a real implementation, this would send to the agent
        let message_id = uuid::Uuid::new_v4().to_string();
        let delta = MessageDelta {
            message_id,
            delta: MessageDeltaContent::Text {
                text: format!("Echo: {}", text),
            },
            timestamp: chrono::Utc::now(),
        };
        
        // Broadcast to all UIs
        let _ = broadcast_tx.send(delta);
        
        Ok(())
    }
    
    /// Unsubscribe from chat for a specific agent
    pub async fn unsubscribe_chat(&self, agent_id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Remove the broadcast channel
        self.chat_broadcasts.write().await.remove(agent_id);
        
        // Cancel any active subscriptions for this agent
        // (In a real implementation, we'd track subscription IDs per agent)
        
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
