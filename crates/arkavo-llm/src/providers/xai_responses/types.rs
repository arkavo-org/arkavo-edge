use crate::common::responses::Usage;
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
    /// Reported `usage` block, read through [`timing_from_usage`].
    pub usage: Option<Value>,
    pub error: Option<Value>,
}

/// Map a Responses `usage` block into [`InferenceTiming`].
///
/// `None` when the block is missing or unreadable: a usage report this crate
/// cannot parse is no reason to fail a completion the caller already paid for.
/// xAI reports `output_tokens` as the total generated (reasoning included), and
/// [`Usage::timing`] keeps the buckets disjoint so cost paths can sum them.
/// A subset larger than its total is clamped rather than rejected, so a bad
/// cache figure cannot read downstream as additional input.
pub(super) fn timing_from_usage(usage: &Value) -> Option<InferenceTiming> {
    Some(Usage::parse(usage).ok()?.clamped().timing())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn timing_excludes_reasoning_from_n_eval() {
        let timing = timing_from_usage(&json!({
            "input_tokens": 100,
            "output_tokens": 50,
            "output_tokens_details": {"reasoning_tokens": 30},
            "input_tokens_details": {"cached_tokens": 80}
        }))
        .expect("a readable usage block maps to timing");
        assert_eq!(timing.n_prompt_eval, 100);
        assert_eq!(timing.n_eval, 20, "visible output must exclude reasoning");
        assert_eq!(timing.n_thinking_eval, Some(30));
        assert_eq!(
            timing.n_cached_prompt_eval,
            Some(80),
            "cached input tokens are billed at the cache rate and must be reported"
        );
        // Downstream cost paths sum these without double-count.
        assert_eq!(
            timing.n_eval + timing.n_thinking_eval.unwrap_or(0),
            50,
            "n_eval + n_thinking_eval must equal reported output_tokens"
        );
    }

    /// A cache figure larger than the reported input would make the cached
    /// tokens look like additional input downstream.
    #[test]
    fn cached_tokens_stay_a_subset_of_the_reported_input() {
        let timing = timing_from_usage(&json!({
            "input_tokens": 40,
            "output_tokens": 5,
            "input_tokens_details": {"cached_tokens": 100}
        }))
        .unwrap();
        assert_eq!(timing.n_cached_prompt_eval, Some(40));
    }

    #[test]
    fn timing_without_reasoning_details() {
        let timing = timing_from_usage(&json!({"input_tokens": 10, "output_tokens": 5})).unwrap();
        assert_eq!(timing.n_eval, 5);
        assert_eq!(timing.n_thinking_eval, None);
        assert_eq!(timing.n_cached_prompt_eval, None);
    }

    /// A completion is not worth failing over an unreadable usage report.
    #[test]
    fn an_unreadable_usage_block_reports_no_timing() {
        assert!(timing_from_usage(&json!({"input_tokens": "many"})).is_none());
        assert!(timing_from_usage(&json!(null)).is_none());
    }
}
