use crate::types::*;
use serde_json::Value;
use std::collections::HashMap;

/// Handles the connection state for a WebSocket client
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

    /// Clean up resources when connection is closed
    pub async fn cleanup(&mut self) {
        // Clear messages and state
        self.messages.clear();
        self.state.clear();
    }
}
