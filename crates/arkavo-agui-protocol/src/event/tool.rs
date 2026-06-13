use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallStartFields {
    pub tool_call_id: String,
    pub tool_call_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallArgsFields {
    pub tool_call_id: String,
    pub delta: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallEndFields {
    pub tool_call_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallResultFields {
    pub message_id: String,
    pub tool_call_id: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallChunkFields {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
}
