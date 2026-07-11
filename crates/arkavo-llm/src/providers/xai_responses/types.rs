use crate::provider::InferenceTiming;
use crate::tool_parser::ParsedToolCall;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Result of a non-streaming Responses call, including multi-turn state.
///
/// `finish_reason` is the Responses `status` field (e.g. `"completed"`), not
/// an OpenAI Chat Completions finish reason like `"tool_calls"`. Tool loops
/// should key off `tool_calls` content rather than this status string.
#[derive(Debug, Clone)]
pub struct ResponsesResult {
    pub response_id: String,
    pub content: String,
    pub reasoning_content: Option<String>,
    pub tool_calls: Vec<ParsedToolCall>,
    pub finish_reason: Option<String>,
    pub inference_timing: Option<InferenceTiming>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ResponsesRequest {
    pub model: String,
    pub input: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ResponsesApiResponse {
    pub id: Option<String>,
    pub status: Option<String>,
    pub output: Option<Vec<Value>>,
    pub usage: Option<ResponsesUsage>,
    pub error: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ResponsesUsage {
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    #[serde(default)]
    pub output_tokens_details: Option<OutputTokenDetails>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OutputTokenDetails {
    pub reasoning_tokens: Option<u32>,
}

pub(super) fn timing_from_usage(usage: &ResponsesUsage) -> InferenceTiming {
    InferenceTiming {
        n_prompt_eval: usage.input_tokens.unwrap_or(0),
        n_eval: usage.output_tokens.unwrap_or(0),
        n_thinking_eval: usage
            .output_tokens_details
            .as_ref()
            .and_then(|d| d.reasoning_tokens),
        ..Default::default()
    }
}
