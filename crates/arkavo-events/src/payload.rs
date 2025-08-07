use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventPayload {
    PromptSent {
        prompt: String,
        model: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        parameters: Option<HashMap<String, Value>>,
    },
    ModelResponse {
        model: String,
        response: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<UsageInfo>,
        duration_ms: u64,
    },
    ToolCall {
        tool_name: String,
        parameters: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_call_id: Option<String>,
    },
    ToolResult {
        tool_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_call_id: Option<String>,
        success: bool,
        result: Value,
        duration_ms: u64,
    },
    FileOperation {
        operation: FileOp,
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        content_preview: Option<String>,
        success: bool,
    },
    ReasoningStep {
        step_type: String,
        description: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<Value>,
    },
    StreamDelta {
        stream_id: String,
        sequence: u64,
        delta_type: String,
        content: String,
    },
    Error {
        error_type: String,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        stack_trace: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        recoverable: Option<bool>,
    },
    SessionStarted {
        #[serde(skip_serializing_if = "Option::is_none")]
        capabilities: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<HashMap<String, Value>>,
    },
    SessionEnded {
        reason: String,
        duration_ms: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<SessionSummary>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageInfo {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOp {
    Read,
    Write,
    Edit,
    Delete,
    Create,
    Rename,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub total_events: u64,
    pub total_prompts: u32,
    pub total_tool_calls: u32,
    pub total_errors: u32,
    pub total_tokens_used: Option<u32>,
}
