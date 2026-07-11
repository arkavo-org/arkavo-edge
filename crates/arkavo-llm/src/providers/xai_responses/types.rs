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

/// Map xAI Responses `usage` into [`InferenceTiming`].
///
/// xAI reports `output_tokens` as the **total** generated tokens (including
/// reasoning). [`InferenceTiming`] keeps `n_eval` and `n_thinking_eval`
/// disjoint so downstream cost paths can sum them without double-counting.
pub(super) fn timing_from_usage(usage: &ResponsesUsage) -> InferenceTiming {
    let reasoning = usage
        .output_tokens_details
        .as_ref()
        .and_then(|d| d.reasoning_tokens);
    let total_output = usage.output_tokens.unwrap_or(0);
    InferenceTiming {
        n_prompt_eval: usage.input_tokens.unwrap_or(0),
        n_eval: total_output.saturating_sub(reasoning.unwrap_or(0)),
        n_thinking_eval: reasoning,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_excludes_reasoning_from_n_eval() {
        let usage = ResponsesUsage {
            input_tokens: Some(100),
            output_tokens: Some(50),
            output_tokens_details: Some(OutputTokenDetails {
                reasoning_tokens: Some(30),
            }),
        };
        let timing = timing_from_usage(&usage);
        assert_eq!(timing.n_prompt_eval, 100);
        assert_eq!(timing.n_eval, 20, "visible output must exclude reasoning");
        assert_eq!(timing.n_thinking_eval, Some(30));
        // Downstream cost paths sum these without double-count.
        assert_eq!(
            timing.n_eval + timing.n_thinking_eval.unwrap_or(0),
            50,
            "n_eval + n_thinking_eval must equal reported output_tokens"
        );
    }

    #[test]
    fn timing_without_reasoning_details() {
        let usage = ResponsesUsage {
            input_tokens: Some(10),
            output_tokens: Some(5),
            output_tokens_details: None,
        };
        let timing = timing_from_usage(&usage);
        assert_eq!(timing.n_eval, 5);
        assert_eq!(timing.n_thinking_eval, None);
    }
}
