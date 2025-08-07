use crate::types::{AgUiEvent, SubscriptionHandle};
use arkavo_debugger::SessionManager;
use arkavo_events::writer::EventWriterBuilder;
use arkavo_events::{Event, EventWriter, EventWriterConfig};
use arkavo_memory::storage::MemoryStorage;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, mpsc};

/// Handles debug event streaming and storage
pub struct DebugHandler {
    storage: Arc<MemoryStorage>,
    event_writer: Arc<EventWriter>,
    subscriptions: Arc<RwLock<Vec<DebugSubscription>>>,
    session_manager: Arc<SessionManager>,
}

struct DebugSubscription {
    session_id: String,
    tx: mpsc::Sender<AgUiEvent>,
    _handle: SubscriptionHandle,
}

impl DebugHandler {
    pub async fn new(storage: Arc<MemoryStorage>) -> Self {
        // Configure event writer with batch flushing
        let config = EventWriterConfig {
            buffer_size: 10_000,
            flush_interval: Duration::from_millis(100),
            batch_size: 200,
        };

        let storage_clone = storage.clone();
        let event_writer = EventWriterBuilder::new()
            .with_config(config)
            .add_handler(move |events| {
                let storage = storage_clone.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_event_batch(storage, events).await {
                        eprintln!("Failed to store event batch: {e}");
                    }
                });
            })
            .build();

        Self {
            storage,
            event_writer: Arc::new(event_writer),
            subscriptions: Arc::new(RwLock::new(Vec::new())),
            session_manager: Arc::new(SessionManager::new()),
        }
    }

    /// Subscribe to debug events for a specific session
    pub async fn subscribe_to_session(
        &self,
        session_id: String,
        tx: mpsc::Sender<AgUiEvent>,
    ) -> SubscriptionHandle {
        let (handle, _rx) = SubscriptionHandle::new(uuid::Uuid::new_v4().to_string());

        let subscription = DebugSubscription {
            session_id: session_id.clone(),
            tx: tx.clone(),
            _handle: handle,
        };

        // Send initial events for the session
        if let Ok(events) = self
            .storage
            .get_session_events(&session_id, Some(100))
            .await
        {
            for stored_event in events {
                if let Ok(event) = convert_stored_to_agui_event(stored_event) {
                    let _ = tx.send(event).await;
                }
            }
        }

        let (new_handle, _) = SubscriptionHandle::new(uuid::Uuid::new_v4().to_string());
        self.subscriptions.write().await.push(subscription);

        new_handle
    }

    /// Write an event to the debug system
    pub async fn write_event(&self, event: Event) -> Result<(), arkavo_events::EventError> {
        let session_id = event.session_id.clone();

        // Store the event
        self.event_writer.write(event.clone()).await?;

        // Broadcast to subscribers
        let subscriptions = self.subscriptions.read().await;
        for sub in subscriptions.iter() {
            if sub.session_id == session_id {
                if let Ok(agui_event) = convert_to_agui_event(&event) {
                    let _ = sub.tx.send(agui_event).await;
                }
            }
        }

        Ok(())
    }

    /// Register a new agent session for debugging
    pub async fn register_session(
        &self,
        session_id: String,
        agent_id: String,
        agent_name: String,
        endpoint: String,
        capabilities: Vec<String>,
    ) -> Result<(), anyhow::Error> {
        self.session_manager
            .register_session(
                session_id,
                agent_id,
                agent_name,
                endpoint,
                capabilities,
                Some(self.event_writer.clone()),
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to register session: {e}"))?;

        Ok(())
    }

    /// Get active sessions from SessionManager
    pub async fn get_active_sessions(&self) -> Vec<arkavo_debugger::SessionInfo> {
        self.session_manager.get_active_sessions().await
    }

    /// Get recent sessions with event counts
    pub async fn get_recent_sessions(
        &self,
        limit: usize,
    ) -> Result<Vec<SessionInfo>, anyhow::Error> {
        // Get active sessions from SessionManager
        let active_sessions = self.session_manager.get_active_sessions().await;

        // Convert to local SessionInfo format
        let mut sessions: Vec<SessionInfo> = active_sessions
            .into_iter()
            .take(limit)
            .map(|s| SessionInfo {
                session_id: s.session_id,
                start_time: s.start_time,
                event_count: s.event_count,
                agent_id: s.agent_id,
            })
            .collect();

        // If we need more sessions, query from storage
        if sessions.len() < limit {
            let (session_count, _) = self.storage.get_event_stats().await?;

            // Add a placeholder for stored sessions (for now)
            if session_count > 0 && sessions.is_empty() {
                sessions.push(SessionInfo {
                    session_id: "stored-sessions".to_string(),
                    start_time: chrono::Utc::now(),
                    event_count: session_count,
                    agent_id: "historical".to_string(),
                });
            }
        }

        Ok(sessions)
    }
}

#[derive(serde::Serialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub event_count: u64,
    pub agent_id: String,
}

async fn handle_event_batch(
    storage: Arc<MemoryStorage>,
    events: Vec<Event>,
) -> Result<(), anyhow::Error> {
    use arkavo_memory::event_store::SerializedEvent;

    let serialized_events: Vec<SerializedEvent> = events
        .into_iter()
        .map(|event| {
            let event_type = event.event_type().to_string();
            let payload = serde_json::to_vec(&event.payload).unwrap_or_default();
            SerializedEvent {
                id: event.id.to_string(),
                session_id: event.session_id,
                sequence: event.sequence,
                timestamp: event.timestamp.to_rfc3339(),
                agent_id: event.metadata.agent_id,
                event_type,
                payload,
                schema_version: event.metadata.schema_version,
            }
        })
        .collect();

    storage.store_events(serialized_events).await?;
    Ok(())
}

fn convert_to_agui_event(event: &Event) -> Result<AgUiEvent, anyhow::Error> {
    use arkavo_events::EventPayload;

    let event_id = event.id.to_string();

    match &event.payload {
        EventPayload::PromptSent { prompt, model, .. } => Ok(AgUiEvent::MessageDelta {
            agent_id: event.metadata.agent_id.clone(),
            message_id: event_id,
            delta: crate::types::MessageDeltaContent::Text {
                text: format!("[Prompt to {model}]: {prompt}"),
            },
        }),
        EventPayload::ModelResponse {
            response, model, ..
        } => Ok(AgUiEvent::MessageDelta {
            agent_id: event.metadata.agent_id.clone(),
            message_id: event_id,
            delta: crate::types::MessageDeltaContent::Text {
                text: format!("[Response from {model}]: {response}"),
            },
        }),
        EventPayload::ToolCall {
            tool_name,
            parameters,
            tool_call_id,
        } => Ok(AgUiEvent::ToolCall {
            tool_call_id: tool_call_id.clone().unwrap_or_else(|| event_id.clone()),
            tool_name: tool_name.clone(),
            arguments: parameters.clone(),
        }),
        EventPayload::Error { message, .. } => Ok(AgUiEvent::Error {
            code: "DEBUG_ERROR".to_string(),
            message: message.clone(),
        }),
        _ => {
            // For other event types, create a generic state delta
            Ok(AgUiEvent::StateDelta {
                patch: vec![],
                event_id,
            })
        }
    }
}

fn convert_stored_to_agui_event(
    stored: arkavo_memory::event_store::StoredEvent,
) -> Result<AgUiEvent, anyhow::Error> {
    // This is a simplified version - in production, we'd deserialize the payload
    // and convert it properly
    Ok(AgUiEvent::StateDelta {
        patch: vec![],
        event_id: stored.id,
    })
}
