use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// AG-UI Event types as per specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AgUiEvent {
    // Outbound events (frontend → agent)
    Connect {
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(rename = "aguiVersion")]
        agui_version: String,
        #[serde(rename = "sinceEventId", skip_serializing_if = "Option::is_none")]
        since_event_id: Option<String>,
    },
    UserMessage {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        attachments: Option<Vec<Attachment>>,
    },
    ToolResult {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        result: Value,
    },
    UiAction {
        action: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        params: Option<Value>,
    },

    // Inbound events (agent → frontend)
    StateSnapshot {
        state: Value,
        #[serde(rename = "eventId")]
        event_id: String,
    },
    MessagesSnapshot {
        messages: Vec<Message>,
        #[serde(rename = "eventId")]
        event_id: String,
    },
    MessageDelta {
        #[serde(rename = "messageId")]
        message_id: String,
        delta: MessageDeltaContent,
    },
    ToolCall {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        arguments: Value,
    },
    StateDelta {
        patch: Vec<JsonPatch>,
        #[serde(rename = "eventId")]
        event_id: String,
    },
    TypingStart {
        #[serde(rename = "messageId")]
        message_id: String,
    },
    TypingStop {
        #[serde(rename = "messageId")]
        message_id: String,
    },

    // Lifecycle events
    #[serde(rename = "lifecycle.start")]
    LifecycleStart {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    #[serde(rename = "lifecycle.end")]
    LifecycleEnd {
        reason: String,
    },
    #[serde(rename = "lifecycle.error")]
    LifecycleError {
        code: String,
        message: String,
    },

    // Error event
    Error {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    #[serde(rename = "type")]
    pub attachment_type: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MessageDeltaContent {
    Text {
        text: String,
    },
    ToolCall {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        delta: String,
    },
}

/// JSON Patch operation as per RFC 6902
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum JsonPatch {
    Add { path: String, value: Value },
    Remove { path: String },
    Replace { path: String, value: Value },
    Move { from: String, path: String },
    Copy { from: String, path: String },
    Test { path: String, value: Value },
}
