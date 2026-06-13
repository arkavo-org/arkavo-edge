//! Convert canonical AG-UI `BaseEvent`s back into the legacy `AgUiEvent` shape
//! used by the existing vanilla-JS frontend on `/ws`.

use arkavo_agui_protocol::{BaseEvent, EventPayload};

use crate::types::{AgUiEvent, Message, MessageDeltaContent, MessageRole, ToolCall};

/// Bridge a canonical event into its legacy equivalent, if one exists.
pub fn bridge(event: BaseEvent) -> Option<AgUiEvent> {
    match event.payload {
        EventPayload::RunStarted(fields) => Some(AgUiEvent::LifecycleStart {
            session_id: fields.run_id,
        }),
        EventPayload::RunFinished(_) => Some(AgUiEvent::LifecycleEnd {
            reason: "complete".into(),
        }),
        EventPayload::RunError(fields) => Some(AgUiEvent::LifecycleError {
            code: fields.code.unwrap_or_default(),
            message: fields.message,
        }),
        EventPayload::TextMessageStart(fields) => Some(AgUiEvent::TypingStart {
            message_id: fields.message_id,
        }),
        EventPayload::TextMessageContent(fields) => Some(AgUiEvent::MessageDelta {
            agent_id: String::new(),
            message_id: fields.message_id,
            delta: MessageDeltaContent::Text { text: fields.delta },
        }),
        EventPayload::TextMessageEnd(fields) => Some(AgUiEvent::TypingStop {
            message_id: fields.message_id,
        }),
        EventPayload::ToolCallStart(fields) => Some(AgUiEvent::MessageDelta {
            agent_id: String::new(),
            message_id: fields.parent_message_id.unwrap_or_default(),
            delta: MessageDeltaContent::ToolCall {
                tool_call_id: fields.tool_call_id,
                name: Some(fields.tool_call_name),
                args_json_fragment: String::new(),
                done: false,
            },
        }),
        EventPayload::ToolCallArgs(fields) => Some(AgUiEvent::MessageDelta {
            agent_id: String::new(),
            message_id: String::new(),
            delta: MessageDeltaContent::ToolCall {
                tool_call_id: fields.tool_call_id,
                name: None,
                args_json_fragment: fields.delta,
                done: false,
            },
        }),
        EventPayload::ToolCallEnd(fields) => Some(AgUiEvent::MessageDelta {
            agent_id: String::new(),
            message_id: String::new(),
            delta: MessageDeltaContent::ToolCall {
                tool_call_id: fields.tool_call_id,
                name: None,
                args_json_fragment: String::new(),
                done: true,
            },
        }),
        EventPayload::ToolCallResult(fields) => Some(AgUiEvent::ToolResult {
            tool_call_id: fields.tool_call_id,
            result: serde_json::Value::String(fields.content),
        }),
        EventPayload::StateSnapshot(fields) => Some(AgUiEvent::StateSnapshot {
            state: fields.snapshot,
            event_id: uuid::Uuid::new_v4().to_string(),
        }),
        EventPayload::MessagesSnapshot(fields) => Some(AgUiEvent::MessagesSnapshot {
            messages: fields
                .messages
                .into_iter()
                .filter_map(canonical_to_legacy_message)
                .collect(),
            event_id: uuid::Uuid::new_v4().to_string(),
        }),
        EventPayload::StateDelta(fields) => Some(AgUiEvent::StateDelta {
            patch: fields.delta.into_iter().map(convert_patch).collect(),
            event_id: uuid::Uuid::new_v4().to_string(),
        }),
        EventPayload::Custom(fields) => bridge_custom(&fields.name, fields.value),
        _ => None,
    }
}

fn convert_patch(patch: arkavo_agui_protocol::JsonPatch) -> crate::types::JsonPatch {
    serde_json::from_value(serde_json::to_value(patch).expect("JsonPatch serializes"))
        .expect("JsonPatch deserializes")
}

fn bridge_custom(name: &str, value: serde_json::Value) -> Option<AgUiEvent> {
    match name {
        "arkavo.model_selected" => Some(AgUiEvent::ModelSelected {
            agent_id: value
                .get("agent_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .into(),
            provider: value
                .get("provider")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .into(),
            model: value
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .into(),
            estimated_cost: serde_json::from_value(value.get("estimated_cost")?.clone()).ok()?,
            reason: value
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .into(),
            event_id: uuid::Uuid::new_v4().to_string(),
        }),
        "arkavo.telemetry" => Some(AgUiEvent::TelemetryEvent {
            event_type: value
                .get("event_type")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .into(),
            agent_id: value
                .get("agent_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .into(),
            details: value
                .get("details")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            timestamp: value
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .into(),
        }),
        "arkavo.arp_status" => serde_json::from_value::<crate::types::ArpStatusSnapshot>(value)
            .ok()
            .map(|snapshot| AgUiEvent::ArpStatusUpdate {
                snapshot,
                event_id: uuid::Uuid::new_v4().to_string(),
            }),
        _ => None,
    }
}

fn canonical_to_legacy_message(msg: arkavo_agui_protocol::message::Message) -> Option<Message> {
    use arkavo_agui_protocol::message::Message as CanonicalMessage;
    match msg {
        CanonicalMessage::User(m) => {
            let content = match m.content {
                arkavo_agui_protocol::message::UserMessageContent::Text(t) => t,
                arkavo_agui_protocol::message::UserMessageContent::Parts(_) => String::new(),
            };
            Some(Message {
                id: m.id,
                role: MessageRole::User,
                content,
                tool_calls: None,
                metadata: None,
            })
        }
        CanonicalMessage::Assistant(m) => Some(Message {
            id: m.id,
            role: MessageRole::Assistant,
            content: m.content.unwrap_or_default(),
            tool_calls: m.tool_calls.map(|calls| {
                calls
                    .into_iter()
                    .map(|c| ToolCall {
                        id: c.id,
                        name: c.function.name,
                        arguments: serde_json::from_str(&c.function.arguments)
                            .unwrap_or(serde_json::Value::Null),
                    })
                    .collect()
            }),
            metadata: None,
        }),
        CanonicalMessage::System(m) => Some(Message {
            id: m.id,
            role: MessageRole::System,
            content: m.content,
            tool_calls: None,
            metadata: None,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use arkavo_agui_protocol::event::{
        RunErrorFields, RunFinishedFields, RunStartedFields, TextMessageContentFields,
        TextMessageEndFields, TextMessageStartFields,
    };
    use arkavo_agui_protocol::message::{AssistantMessage, UserMessage, UserMessageContent};
    use arkavo_test_macros::spec;

    use super::*;

    #[test]
    #[spec("AGUI-019")]
    fn lifecycle_bridge() {
        let started = bridge(BaseEvent::new(EventPayload::RunStarted(RunStartedFields {
            thread_id: "t".into(),
            run_id: "r".into(),
            parent_run_id: None,
            input: None,
        })));
        assert!(matches!(started, Some(AgUiEvent::LifecycleStart { .. })));

        let err = bridge(BaseEvent::new(EventPayload::RunError(RunErrorFields {
            thread_id: "t".into(),
            run_id: "r".into(),
            message: "boom".into(),
            code: Some("E".into()),
        })));
        assert!(matches!(err, Some(AgUiEvent::LifecycleError { .. })));
    }

    #[test]
    #[spec("AGUI-019")]
    fn text_message_bridge() {
        let start = bridge(BaseEvent::new(EventPayload::TextMessageStart(
            TextMessageStartFields {
                message_id: "m1".into(),
                role: "assistant".into(),
            },
        )));
        assert!(matches!(start, Some(AgUiEvent::TypingStart { .. })));

        let content = bridge(BaseEvent::new(EventPayload::TextMessageContent(
            TextMessageContentFields {
                message_id: "m1".into(),
                delta: "hello".into(),
            },
        )));
        assert!(matches!(content, Some(AgUiEvent::MessageDelta { .. })));

        let end = bridge(BaseEvent::new(EventPayload::TextMessageEnd(
            TextMessageEndFields {
                message_id: "m1".into(),
            },
        )));
        assert!(matches!(end, Some(AgUiEvent::TypingStop { .. })));
    }

    #[test]
    #[spec("AGUI-019")]
    fn messages_snapshot_bridge() {
        let snapshot = BaseEvent::new(EventPayload::MessagesSnapshot(
            arkavo_agui_protocol::event::MessagesSnapshotFields {
                messages: vec![
                    arkavo_agui_protocol::message::Message::User(UserMessage {
                        id: "u1".into(),
                        content: UserMessageContent::Text("hi".into()),
                        name: None,
                        encrypted_value: None,
                    }),
                    arkavo_agui_protocol::message::Message::Assistant(AssistantMessage {
                        id: "a1".into(),
                        content: Some("hello".into()),
                        tool_calls: None,
                        name: None,
                        encrypted_value: None,
                    }),
                ],
            },
        ));
        let legacy = bridge(snapshot);
        assert!(matches!(legacy, Some(AgUiEvent::MessagesSnapshot { .. })));
    }
}
