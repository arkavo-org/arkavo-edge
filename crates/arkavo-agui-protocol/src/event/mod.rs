//! AG-UI `BaseEvent` and per-category payload types.

pub mod activity;
pub mod deprecated;
pub mod lifecycle;
pub mod reasoning;
pub mod special;
pub mod state;
pub mod text;
pub mod tool;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use activity::*;
pub use deprecated::*;
pub use lifecycle::*;
pub use reasoning::*;
pub use special::*;
pub use state::*;
pub use text::*;
pub use tool::*;

/// Every AG-UI event on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaseEvent {
    #[serde(flatten)]
    pub payload: EventPayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "rawEvent")]
    pub raw_event: Option<Value>,
}

/// Discriminated union of all spec-defined event payloads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventPayload {
    // Lifecycle
    RunStarted(RunStartedFields),
    RunFinished(RunFinishedFields),
    RunError(RunErrorFields),
    StepStarted(StepStartedFields),
    StepFinished(StepFinishedFields),

    // Text messages
    TextMessageStart(TextMessageStartFields),
    TextMessageContent(TextMessageContentFields),
    TextMessageEnd(TextMessageEndFields),
    TextMessageChunk(TextMessageChunkFields),

    // Tool calls
    ToolCallStart(ToolCallStartFields),
    ToolCallArgs(ToolCallArgsFields),
    ToolCallEnd(ToolCallEndFields),
    ToolCallResult(ToolCallResultFields),
    ToolCallChunk(ToolCallChunkFields),

    // State
    StateSnapshot(StateSnapshotFields),
    StateDelta(StateDeltaFields),
    MessagesSnapshot(MessagesSnapshotFields),

    // Activity
    ActivitySnapshot(ActivitySnapshotFields),
    ActivityDelta(ActivityDeltaFields),

    // Reasoning
    ReasoningStart(ReasoningStartFields),
    ReasoningMessageStart(ReasoningMessageStartFields),
    ReasoningMessageContent(ReasoningMessageContentFields),
    ReasoningMessageEnd(ReasoningMessageEndFields),
    ReasoningMessageChunk(ReasoningMessageChunkFields),
    ReasoningEnd(ReasoningEndFields),
    ReasoningEncryptedValue(ReasoningEncryptedValueFields),

    // Special
    Raw(RawFields),
    Custom(CustomFields),

    // Deprecated thinking events (kept for backward compatibility).
    ThinkingStart(ThinkingStartFields),
    ThinkingEnd(ThinkingEndFields),
    ThinkingTextMessageStart(ThinkingTextMessageStartFields),
    ThinkingTextMessageContent(ThinkingTextMessageContentFields),
    ThinkingTextMessageEnd(ThinkingTextMessageEndFields),
}

impl BaseEvent {
    /// Build an event with the current Unix timestamp.
    pub fn now(payload: EventPayload) -> Self {
        Self {
            payload,
            timestamp: Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system clock before epoch")
                    .as_secs(),
            ),
            raw_event: None,
        }
    }

    /// Build an event without a timestamp (useful for deterministic tests).
    pub fn new(payload: EventPayload) -> Self {
        Self {
            payload,
            timestamp: None,
            raw_event: None,
        }
    }

    /// Return the event type string.
    pub fn event_type(&self) -> &'static str {
        match self.payload {
            EventPayload::RunStarted(_) => "RUN_STARTED",
            EventPayload::RunFinished(_) => "RUN_FINISHED",
            EventPayload::RunError(_) => "RUN_ERROR",
            EventPayload::StepStarted(_) => "STEP_STARTED",
            EventPayload::StepFinished(_) => "STEP_FINISHED",
            EventPayload::TextMessageStart(_) => "TEXT_MESSAGE_START",
            EventPayload::TextMessageContent(_) => "TEXT_MESSAGE_CONTENT",
            EventPayload::TextMessageEnd(_) => "TEXT_MESSAGE_END",
            EventPayload::TextMessageChunk(_) => "TEXT_MESSAGE_CHUNK",
            EventPayload::ToolCallStart(_) => "TOOL_CALL_START",
            EventPayload::ToolCallArgs(_) => "TOOL_CALL_ARGS",
            EventPayload::ToolCallEnd(_) => "TOOL_CALL_END",
            EventPayload::ToolCallResult(_) => "TOOL_CALL_RESULT",
            EventPayload::ToolCallChunk(_) => "TOOL_CALL_CHUNK",
            EventPayload::StateSnapshot(_) => "STATE_SNAPSHOT",
            EventPayload::StateDelta(_) => "STATE_DELTA",
            EventPayload::MessagesSnapshot(_) => "MESSAGES_SNAPSHOT",
            EventPayload::ActivitySnapshot(_) => "ACTIVITY_SNAPSHOT",
            EventPayload::ActivityDelta(_) => "ACTIVITY_DELTA",
            EventPayload::ReasoningStart(_) => "REASONING_START",
            EventPayload::ReasoningMessageStart(_) => "REASONING_MESSAGE_START",
            EventPayload::ReasoningMessageContent(_) => "REASONING_MESSAGE_CONTENT",
            EventPayload::ReasoningMessageEnd(_) => "REASONING_MESSAGE_END",
            EventPayload::ReasoningMessageChunk(_) => "REASONING_MESSAGE_CHUNK",
            EventPayload::ReasoningEnd(_) => "REASONING_END",
            EventPayload::ReasoningEncryptedValue(_) => "REASONING_ENCRYPTED_VALUE",
            EventPayload::Raw(_) => "RAW",
            EventPayload::Custom(_) => "CUSTOM",
            EventPayload::ThinkingStart(_) => "THINKING_START",
            EventPayload::ThinkingEnd(_) => "THINKING_END",
            EventPayload::ThinkingTextMessageStart(_) => "THINKING_TEXT_MESSAGE_START",
            EventPayload::ThinkingTextMessageContent(_) => "THINKING_TEXT_MESSAGE_CONTENT",
            EventPayload::ThinkingTextMessageEnd(_) => "THINKING_TEXT_MESSAGE_END",
        }
    }
}
