use serde_json::json;

use crate::BaseEvent;
use crate::event::*;
use crate::input::*;
use crate::message::*;
use crate::patch::JsonPatch;

#[test]
fn lifecycle_events_round_trip() {
    let started = BaseEvent::new(EventPayload::RunStarted(RunStartedFields {
        thread_id: "thread-1".into(),
        run_id: "run-1".into(),
        parent_run_id: Some("run-0".into()),
        input: None,
    }));
    let json = serde_json::to_string(&started).unwrap();
    assert!(json.contains("\"type\":\"RUN_STARTED\""));
    assert!(json.contains("\"threadId\":\"thread-1\""));
    let back: BaseEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(back.payload, started.payload);

    let finished = BaseEvent::new(EventPayload::RunFinished(RunFinishedFields {
        thread_id: "thread-1".into(),
        run_id: "run-1".into(),
        outcome: Some(RunOutcome::Success),
        result: Some(json!({"ok": true})),
    }));
    let json = serde_json::to_string(&finished).unwrap();
    assert!(json.contains("\"type\":\"RUN_FINISHED\""));
    assert!(json.contains("\"type\":\"success\""));
    let back: BaseEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(back.payload, finished.payload);

    let err = BaseEvent::new(EventPayload::RunError(RunErrorFields {
        thread_id: "thread-1".into(),
        run_id: "run-1".into(),
        message: "boom".into(),
        code: Some("E1".into()),
    }));
    assert_round_trip(&err);

    let step = BaseEvent::new(EventPayload::StepStarted(StepStartedFields {
        thread_id: "thread-1".into(),
        run_id: "run-1".into(),
        step_name: "plan".into(),
    }));
    assert_round_trip(&step);
}

#[test]
fn text_message_stream_round_trip() {
    let start = BaseEvent::new(EventPayload::TextMessageStart(TextMessageStartFields {
        message_id: "msg-1".into(),
        role: "assistant".into(),
    }));
    let content = BaseEvent::new(EventPayload::TextMessageContent(TextMessageContentFields {
        message_id: "msg-1".into(),
        delta: "hello".into(),
    }));
    let end = BaseEvent::new(EventPayload::TextMessageEnd(TextMessageEndFields {
        message_id: "msg-1".into(),
    }));

    for ev in [&start, &content, &end] {
        assert_round_trip(ev);
    }

    let chunk = BaseEvent::new(EventPayload::TextMessageChunk(TextMessageChunkFields {
        message_id: Some("msg-1".into()),
        role: Some("assistant".into()),
        delta: Some("chunk".into()),
    }));
    assert_round_trip(&chunk);
}

#[test]
fn tool_call_stream_round_trip() {
    let start = BaseEvent::new(EventPayload::ToolCallStart(ToolCallStartFields {
        tool_call_id: "tc-1".into(),
        tool_call_name: "read_file".into(),
        parent_message_id: Some("msg-1".into()),
    }));
    let args = BaseEvent::new(EventPayload::ToolCallArgs(ToolCallArgsFields {
        tool_call_id: "tc-1".into(),
        delta: "{\"path\":\"x\"}".into(),
    }));
    let end = BaseEvent::new(EventPayload::ToolCallEnd(ToolCallEndFields {
        tool_call_id: "tc-1".into(),
    }));
    let result = BaseEvent::new(EventPayload::ToolCallResult(ToolCallResultFields {
        message_id: "msg-1".into(),
        tool_call_id: "tc-1".into(),
        content: "contents".into(),
        role: Some("tool".into()),
    }));

    for ev in [&start, &args, &end, &result] {
        assert_round_trip(ev);
    }
}

#[test]
fn state_and_message_snapshot_round_trip() {
    let state = BaseEvent::new(EventPayload::StateSnapshot(StateSnapshotFields {
        snapshot: json!({"counter": 3}),
    }));
    assert_round_trip(&state);

    let delta = BaseEvent::new(EventPayload::StateDelta(StateDeltaFields {
        delta: vec![JsonPatch::Replace {
            path: "/counter".into(),
            value: json!(4),
        }],
    }));
    assert_round_trip(&delta);

    let messages = BaseEvent::new(EventPayload::MessagesSnapshot(MessagesSnapshotFields {
        messages: vec![
            Message::User(UserMessage {
                id: "u1".into(),
                content: UserMessageContent::Text("hi".into()),
                name: None,
                encrypted_value: None,
            }),
            Message::Assistant(AssistantMessage {
                id: "a1".into(),
                content: Some("hello".into()),
                tool_calls: Some(vec![ToolCall {
                    id: "tc1".into(),
                    r#type: ToolCallType::Function,
                    function: FunctionCall {
                        name: "search".into(),
                        arguments: "{\"q\":\"x\"}".into(),
                    },
                    encrypted_value: None,
                }]),
                name: None,
                encrypted_value: None,
            }),
        ],
    }));
    assert_round_trip(&messages);
}

#[test]
fn activity_and_reasoning_round_trip() {
    let activity = BaseEvent::new(EventPayload::ActivitySnapshot(ActivitySnapshotFields {
        message_id: "act-1".into(),
        activity_type: "PLAN".into(),
        content: json!({"steps": []}),
        replace: Some(true),
    }));
    assert_round_trip(&activity);

    let reasoning = BaseEvent::new(EventPayload::ReasoningStart(ReasoningStartFields {
        message_id: "rsn-1".into(),
    }));
    let rcontent = BaseEvent::new(EventPayload::ReasoningMessageContent(
        ReasoningMessageContentFields {
            message_id: "rsn-1".into(),
            delta: "therefore".into(),
        },
    ));
    let rend = BaseEvent::new(EventPayload::ReasoningEnd(ReasoningEndFields {
        message_id: "rsn-1".into(),
    }));
    for ev in [&reasoning, &rcontent, &rend] {
        assert_round_trip(ev);
    }

    let encrypted = BaseEvent::new(EventPayload::ReasoningEncryptedValue(
        ReasoningEncryptedValueFields {
            subtype: "message".into(),
            entity_id: "msg-1".into(),
            encrypted_value: "vault".into(),
        },
    ));
    assert_round_trip(&encrypted);
}

#[test]
fn special_and_deprecated_events_round_trip() {
    let raw = BaseEvent::new(EventPayload::Raw(RawFields {
        event: json!({"foo": 1}),
        source: Some("external".into()),
    }));
    assert_round_trip(&raw);

    let custom = BaseEvent::new(EventPayload::Custom(CustomFields {
        name: "arkavo.budget_status".into(),
        value: json!({"ok": true}),
    }));
    assert_round_trip(&custom);

    let thinking = BaseEvent::new(EventPayload::ThinkingStart(ThinkingStartFields {
        message_id: "th-1".into(),
    }));
    assert_round_trip(&thinking);
}

#[test]
fn message_roles_serialize_correctly() {
    let user_text = serde_json::to_string(&Message::User(UserMessage {
        id: "u1".into(),
        content: UserMessageContent::Text("hi".into()),
        name: None,
        encrypted_value: None,
    }))
    .unwrap();
    assert!(user_text.contains("\"role\":\"user\""));
    assert!(user_text.contains("\"content\":\"hi\""));

    let assistant = serde_json::to_string(&Message::Assistant(AssistantMessage {
        id: "a1".into(),
        content: Some("hello".into()),
        tool_calls: None,
        name: None,
        encrypted_value: None,
    }))
    .unwrap();
    assert!(assistant.contains("\"role\":\"assistant\""));

    let tool_msg = serde_json::to_string(&Message::Tool(ToolMessage {
        id: "t1".into(),
        content: "result".into(),
        tool_call_id: "tc1".into(),
        error: None,
        encrypted_value: None,
    }))
    .unwrap();
    assert!(tool_msg.contains("\"role\":\"tool\""));
    assert!(tool_msg.contains("\"toolCallId\":\"tc1\""));
}

#[test]
fn multimodal_user_message_round_trip() {
    let msg = Message::User(UserMessage {
        id: "u2".into(),
        content: UserMessageContent::Parts(vec![
            InputContent::Text(TextContent {
                text: "describe".into(),
            }),
            InputContent::Image(ImageContent {
                source: InputContentSource::Url {
                    value: "https://example.com/i.png".into(),
                    mime_type: Some("image/png".into()),
                },
                metadata: None,
            }),
        ]),
        name: None,
        encrypted_value: None,
    });
    let json = serde_json::to_string(&msg).unwrap();
    let back: Message = serde_json::from_str(&json).unwrap();
    assert_eq!(back, msg);
}

#[test]
fn run_agent_input_round_trip() {
    let input = RunAgentInput {
        thread_id: "thread-1".into(),
        run_id: "run-1".into(),
        parent_run_id: None,
        state: json!({"x": 1}),
        messages: vec![Message::User(UserMessage {
            id: "u1".into(),
            content: UserMessageContent::Text("hello".into()),
            name: None,
            encrypted_value: None,
        })],
        tools: vec![Tool::new(
            "search",
            "search the web",
            json!({"type": "object"}),
        )],
        context: vec![Context {
            description: "locale".into(),
            value: "en-US".into(),
        }],
        forwarded_props: json!({}),
        resume: None,
    };
    let json = serde_json::to_string(&input).unwrap();
    let back: RunAgentInput = serde_json::from_str(&json).unwrap();
    assert_eq!(back, input);
}

#[test]
fn capabilities_serialize() {
    let caps = AgentCapabilities::arkavo_default();
    let json = serde_json::to_string(&caps).unwrap();
    assert!(json.contains("\"humanInTheLoop\":true"));
    assert!(json.contains("\"sse\":true"));
}

#[test]
fn event_type_helper_matches_serde_tag() {
    let ev = BaseEvent::new(EventPayload::RunStarted(RunStartedFields {
        thread_id: "t".into(),
        run_id: "r".into(),
        parent_run_id: None,
        input: None,
    }));
    assert_eq!(ev.event_type(), "RUN_STARTED");
}

#[test]
fn base_event_timestamp_and_raw_event_preserved() {
    let ev = BaseEvent {
        payload: EventPayload::Custom(CustomFields {
            name: "x".into(),
            value: json!(1),
        }),
        timestamp: Some(1234567890),
        raw_event: Some(json!({"legacy": true})),
    };
    let json = serde_json::to_string(&ev).unwrap();
    assert!(json.contains("\"timestamp\":1234567890"));
    assert!(json.contains("\"rawEvent\":{\"legacy\":true}"));
    let back: BaseEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(back.timestamp, Some(1234567890));
    assert_eq!(back.raw_event, Some(json!({"legacy": true})));
}

#[test]
fn message_helpers_work() {
    let assistant = Message::Assistant(AssistantMessage {
        id: "a1".into(),
        content: None,
        tool_calls: Some(vec![ToolCall {
            id: "tc".into(),
            r#type: ToolCallType::Function,
            function: FunctionCall {
                name: "f".into(),
                arguments: "{}".into(),
            },
            encrypted_value: None,
        }]),
        name: None,
        encrypted_value: None,
    });
    assert_eq!(assistant.id(), "a1");
    assert_eq!(assistant.tool_calls().unwrap().len(), 1);
}

fn assert_round_trip(ev: &BaseEvent) {
    let json = serde_json::to_string(ev).unwrap();
    let back: BaseEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, ev, "round-trip failed for {json}");
}
