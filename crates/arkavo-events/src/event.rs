use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::SCHEMA_VERSION;
use crate::payload::EventPayload;

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
    /// SEQ-015: taint carried by whatever this event describes, source through
    /// provenance, so a forensic replay can follow data across events without
    /// reconstructing it from payloads. `None` means the producer tracked no
    /// taint for this event, which is not a claim that none applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taint_chain: Option<crate::payload::TaintRecord>,
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
                taint_chain: None,
            },
            payload,
        }
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
            EventPayload::SequenceNode { .. } => "sequence_node",
            EventPayload::SequenceViolation { .. } => "sequence_violation",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::FileOp;
    use serde_json::Value;

    fn make_prompt_payload() -> EventPayload {
        EventPayload::PromptSent {
            prompt: "hello".to_string(),
            model: "test-model".to_string(),
            parameters: None,
        }
    }

    fn make_event() -> Event {
        Event::new(
            "sess-001".to_string(),
            1,
            "agent-1".to_string(),
            make_prompt_payload(),
        )
    }

    #[test]
    fn new_generates_non_nil_uuid() {
        let event = make_event();
        assert!(!event.id.is_nil());
    }

    #[test]
    fn new_has_schema_version_1_0_0() {
        let event = make_event();
        assert_eq!(event.metadata.schema_version, "1.0.0");
    }

    #[test]
    fn new_has_no_parent_event_id() {
        let event = make_event();
        assert!(event.metadata.parent_event_id.is_none());
    }

    #[test]
    fn new_has_no_correlation_id() {
        let event = make_event();
        assert!(event.metadata.correlation_id.is_none());
    }

    #[test]
    fn new_stores_session_id() {
        let event = make_event();
        assert_eq!(event.session_id, "sess-001");
    }

    #[test]
    fn new_stores_sequence() {
        let event = make_event();
        assert_eq!(event.sequence, 1);
    }

    #[test]
    fn event_type_prompt_sent() {
        let event = make_event();
        assert_eq!(event.event_type(), "prompt_sent");
    }

    #[test]
    fn event_type_model_response() {
        let payload = EventPayload::ModelResponse {
            model: "m".to_string(),
            response: "r".to_string(),
            usage: None,
            duration_ms: 100,
        };
        let event = Event::new("s".into(), 0, "a".into(), payload);
        assert_eq!(event.event_type(), "model_response");
    }

    #[test]
    fn event_type_tool_call() {
        let payload = EventPayload::ToolCall {
            tool_name: "read".to_string(),
            parameters: Value::Null,
            tool_call_id: None,
        };
        let event = Event::new("s".into(), 0, "a".into(), payload);
        assert_eq!(event.event_type(), "tool_call");
    }

    #[test]
    fn event_type_tool_result() {
        let payload = EventPayload::ToolResult {
            tool_name: "read".to_string(),
            tool_call_id: None,
            success: true,
            result: Value::Null,
            duration_ms: 50,
        };
        let event = Event::new("s".into(), 0, "a".into(), payload);
        assert_eq!(event.event_type(), "tool_result");
    }

    #[test]
    fn event_type_file_operation() {
        let payload = EventPayload::FileOperation {
            operation: FileOp::Read,
            path: "/tmp/f".to_string(),
            content_preview: None,
            success: true,
        };
        let event = Event::new("s".into(), 0, "a".into(), payload);
        assert_eq!(event.event_type(), "file_operation");
    }

    #[test]
    fn event_type_reasoning_step() {
        let payload = EventPayload::ReasoningStep {
            step_type: "plan".to_string(),
            description: "d".to_string(),
            metadata: None,
        };
        let event = Event::new("s".into(), 0, "a".into(), payload);
        assert_eq!(event.event_type(), "reasoning_step");
    }

    #[test]
    fn event_type_stream_delta() {
        let payload = EventPayload::StreamDelta {
            stream_id: "s1".to_string(),
            sequence: 0,
            delta_type: "text".to_string(),
            content: "c".to_string(),
        };
        let event = Event::new("s".into(), 0, "a".into(), payload);
        assert_eq!(event.event_type(), "stream_delta");
    }

    #[test]
    fn event_type_error() {
        let payload = EventPayload::Error {
            error_type: "runtime".to_string(),
            message: "oops".to_string(),
            stack_trace: None,
            recoverable: None,
        };
        let event = Event::new("s".into(), 0, "a".into(), payload);
        assert_eq!(event.event_type(), "error");
    }

    #[test]
    fn event_type_session_started() {
        let payload = EventPayload::SessionStarted {
            capabilities: None,
            metadata: None,
        };
        let event = Event::new("s".into(), 0, "a".into(), payload);
        assert_eq!(event.event_type(), "session_started");
    }

    #[test]
    fn event_type_session_ended() {
        let payload = EventPayload::SessionEnded {
            reason: "done".to_string(),
            duration_ms: 1000,
            summary: None,
        };
        let event = Event::new("s".into(), 0, "a".into(), payload);
        assert_eq!(event.event_type(), "session_ended");
    }

    #[test]
    fn event_type_sequence_node() {
        let payload = EventPayload::SequenceNode {
            node_id: "n-1".to_string(),
            tool_name: "read_file".to_string(),
            params_hash: "0f".to_string(),
            inputs: Vec::new(),
            taint: crate::payload::TaintRecord {
                sensitivity: "internal".to_string(),
                ..Default::default()
            },
        };
        let event = Event::new("s".into(), 0, "a".into(), payload);
        assert_eq!(event.event_type(), "sequence_node");
    }
}
