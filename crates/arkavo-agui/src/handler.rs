use crate::types::*;
use serde_json::Value;
use std::collections::HashMap;

/// Handles the connection between a WebSocket client and an agent
pub struct ConnectionHandler {
    pub session_id: String,
    pub agent_info: Value,
    pub messages: Vec<Message>,
    pub state: HashMap<String, Value>,
    pub event_counter: u64,
}

impl ConnectionHandler {
    pub fn new(session_id: String, agent_info: Value) -> Self {
        Self {
            session_id,
            agent_info,
            messages: Vec::new(),
            state: HashMap::new(),
            event_counter: 0,
        }
    }

    pub fn next_event_id(&mut self) -> String {
        self.event_counter += 1;
        format!("{}-{}", self.session_id, self.event_counter)
    }

    pub async fn forward_to_agent(
        &self,
        event: AgUiEvent,
    ) -> Result<AgUiEvent, Box<dyn std::error::Error>> {
        // TODO: Implement actual forwarding to agent via JSON-RPC
        // For now, return a mock response
        match event {
            AgUiEvent::UserMessage { content, .. } => Ok(AgUiEvent::MessageDelta {
                message_id: uuid::Uuid::new_v4().to_string(),
                delta: MessageDeltaContent::Text {
                    text: format!("Echo: {}", content),
                },
            }),
            _ => Ok(AgUiEvent::Error {
                code: "NOT_IMPLEMENTED".to_string(),
                message: "Agent forwarding not yet implemented".to_string(),
            }),
        }
    }
}
