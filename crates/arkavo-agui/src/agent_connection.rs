use jsonrpsee::core::client::ClientT;
use jsonrpsee::ws_client::{WsClient, WsClientBuilder};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tokio::time::{Duration, sleep};

/// Represents a persistent connection to an AI agent
#[derive(Clone)]
pub struct AgentConnection {
    agent_id: String,
    endpoint: String,
    client: Arc<RwLock<Option<WsClient>>>,
    status: Arc<RwLock<ConnectionStatus>>,
    telemetry_tx: mpsc::Sender<TelemetryEvent>,
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
}

// Re-export for convenience
pub use jsonrpsee::rpc_params;
