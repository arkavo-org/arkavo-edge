use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::payload::EventPayload;
use crate::SCHEMA_VERSION;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: Uuid,
    pub session_id: String,
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub metadata: EventMetadata,
    pub payload: EventPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMetadata {
    pub agent_id: String,
    pub schema_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_event_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

impl Event {
    pub fn new(session_id: String, sequence: u64, agent_id: String, payload: EventPayload) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_id,
            sequence,
            timestamp: Utc::now(),
            metadata: EventMetadata {
                agent_id,
                schema_version: SCHEMA_VERSION.to_string(),
                parent_event_id: None,
                correlation_id: None,
            },
            payload,
        }
    }

    pub fn with_parent(mut self, parent_id: Uuid) -> Self {
        self.metadata.parent_event_id = Some(parent_id);
        self
    }

    pub fn with_correlation(mut self, correlation_id: String) -> Self {
        self.metadata.correlation_id = Some(correlation_id);
        self
    }

    pub fn event_type(&self) -> &'static str {
        match &self.payload {
            EventPayload::PromptSent { .. } => "prompt_sent",
            EventPayload::ModelResponse { .. } => "model_response",
            EventPayload::ToolCall { .. } => "tool_call",
            EventPayload::ToolResult { .. } => "tool_result",
            EventPayload::FileOperation { .. } => "file_operation",
            EventPayload::ReasoningStep { .. } => "reasoning_step",
            EventPayload::StreamDelta { .. } => "stream_delta",
            EventPayload::Error { .. } => "error",
            EventPayload::SessionStarted { .. } => "session_started",
            EventPayload::SessionEnded { .. } => "session_ended",
        }
    }
}
