use serde::{Deserialize, Serialize};

/// Deprecated `THINKING_START` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingStartFields {
    pub message_id: String,
}

/// Deprecated `THINKING_END` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingEndFields {
    pub message_id: String,
}

/// Deprecated `THINKING_TEXT_MESSAGE_START` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingTextMessageStartFields {
    pub message_id: String,
    pub role: String,
}

/// Deprecated `THINKING_TEXT_MESSAGE_CONTENT` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingTextMessageContentFields {
    pub message_id: String,
    pub delta: String,
}

/// Deprecated `THINKING_TEXT_MESSAGE_END` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingTextMessageEndFields {
    pub message_id: String,
}
