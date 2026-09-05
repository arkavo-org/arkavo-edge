use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_stream::Stream;

use crate::provider_state::ProviderState;
use crate::tool_parser::ParsedToolCall;
use crate::{Message, Result, StreamResponse};

/// Response from a provider that may include tool calls
#[derive(Clone, Default)]
pub struct ProviderResponse {
    /// Exact provider output items for stateless continuation; never display these.
    pub provider_state: ProviderState,
    pub content: String,
    /// Reasoning/thinking content from models with thinking mode (e.g., DeepSeek V3.2-Speciale)
    pub reasoning_content: Option<String>,
    pub tool_calls: Vec<ParsedToolCall>,
    pub finish_reason: Option<String>,
    /// LLM inference timing from provider (prompt_eval_ms, generation_ms, n_p_eval, n_eval)
    pub inference_timing: Option<InferenceTiming>,
    /// Number of quality gate retry attempts (0 = first attempt succeeded)
    pub quality_gate_retries: u8,
}

impl ProviderResponse {
    /// Distinguish provider-native calls from tool syntax extracted from prose.
    pub fn has_native_response_calls(&self) -> bool {
        self.provider_state.has_native_calls()
    }

    /// Whether this turn's tool results must be replayed as `Role::Tool`.
    ///
    /// Chat Completions providers (OpenAI-compatible, GLM, Grok, Anthropic)
    /// carry no provider state at all, yet their assistant `tool_calls` are
    /// native and the API rejects any continuation that answers them with a
    /// user message. Local templates likewise expect tool roles. Only a
    /// Responses turn that returned items without a `function_call` among them
    /// had its calls extracted from prose, and a `function_call_output` cannot
    /// be submitted for a call the provider never recorded.
    pub fn tool_results_use_tool_role(&self) -> bool {
        self.provider_state.is_empty() || self.has_native_response_calls()
    }

    /// Preserve native tool IDs and opaque provider state in the next turn.
    pub fn as_assistant_message(&self) -> Message {
        let mut message = Message::assistant_with_tool_calls(
            self.content.clone(),
            self.tool_calls
                .iter()
                .map(|call| crate::message::ToolCall {
                    name: call.tool_name.clone(),
                    arguments: call.arguments.to_string(),
                    id: call.call_id.clone(),
                })
                .collect(),
        );
        message.provider_state.clone_from(&self.provider_state);
        message
    }
}

/// Timing and token-usage data from the LLM inference engine.
/// Populated by llama.cpp and by cloud providers that surface usage
/// metadata (Gemini 3.5's `usageMetadata` block in particular).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InferenceTiming {
    /// Prompt evaluation / prefill time in ms
    pub prompt_eval_ms: f64,
    /// Token generation time in ms
    pub generation_ms: f64,
    /// Total prompt tokens, including tokens served from the provider cache
    pub n_prompt_eval: u32,
    /// Cached input tokens, a subset of n_prompt_eval (not additional input).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n_cached_prompt_eval: Option<u32>,
    /// Cache-write input tokens, disjoint from cached reads and included in n_prompt_eval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n_cache_write_prompt_eval: Option<u32>,
    /// Number of tokens generated and surfaced to the caller (visible
    /// text + tool calls — for thinking models this excludes the
    /// internal chain-of-thought).
    pub n_eval: u32,
    /// Tokens spent inside the model's hidden chain-of-thought, when
    /// the provider reports it separately (Gemini 3.5's
    /// `thoughtsTokenCount`). These count toward billing on Gemini but
    /// are invisible to the response stream — call them out separately
    /// so the cost path can multiply by output-rate without double-
    /// counting and so latency analysis can attribute spikes correctly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n_thinking_eval: Option<u32>,
    /// Tokens drafted by spec decoding (sum across all draft calls).
    /// None when spec was disabled for the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n_draft: Option<u32>,
    /// Tokens accepted from drafts (n_accepted <= n_draft).
    /// None when spec was disabled for the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n_accepted: Option<u32>,
    /// Set when the caller requested spec decoding but the streaming layer
    /// declined to engage it (e.g., grammar active, stops active). The router
    /// uses this to avoid penalizing the model's accept-rate stats for cases
    /// where spec never had a chance to run. None means either spec wasn't
    /// requested, or spec ran (in which case n_draft/n_accepted are Some).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec_bypassed: Option<String>,
    /// Mean per-token log-probability of the generated answer, when the engine
    /// computes it. Near `0` is confident; very negative means the model was
    /// guessing. Feeds the quality plane's `LowConfidence` adequacy signal.
    /// `None` for providers/paths that don't surface logprobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_logprob: Option<f32>,
}

/// Per-request completion options that the router populates and the
/// provider reads. Today only the local llama.cpp path consumes
/// `use_spec_decoding`; cloud providers ignore unknown fields.
///
/// This struct is intentionally narrow: only carries decisions the
/// router needs to push down per-call (spec on/off, future: cache
/// salt, retrieval budget). Sampling parameters (temperature, top_p,
/// max_tokens) stay on the provider's static config so we don't
/// double-source them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompletionOptions {
    /// Max tokens override; falls back to provider config when None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,
    /// Sampler seed override; falls back to provider config when None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,
    /// Sampler temperature override; falls back to provider config when None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Enable NGRAM self-speculative decoding for this request. Router
    /// decides per-model based on rolling accept-rate stats. Default
    /// false (caller opts in explicitly).
    #[serde(default)]
    pub use_spec_decoding: bool,
}

#[async_trait]
pub trait Provider: Send + Sync {
    async fn complete(&self, messages: Vec<Message>) -> Result<String> {
        self.complete_with_options(messages, None).await
    }

    async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        max_tokens: Option<usize>,
    ) -> Result<String>;

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>>;

    fn name(&self) -> &str;

    /// Check if this provider supports native tool calling
    fn supports_tools(&self) -> bool {
        false
    }

    /// Complete with tool support (returns structured response with tool calls)
    /// Default implementation returns response without tool calls for backward compatibility
    async fn complete_with_tools(
        &self,
        messages: Vec<Message>,
        _tools: Option<Value>,
        max_tokens: Option<usize>,
    ) -> Result<ProviderResponse> {
        let content = self.complete_with_options(messages, max_tokens).await?;
        Ok(ProviderResponse {
            content,
            ..Default::default()
        })
    }

    /// Check if this provider supports structured JSON output with schema
    fn supports_structured_output(&self) -> bool {
        false
    }

    /// Complete with a JSON schema for structured output
    /// Providers that support structured output (e.g., Gemini) will constrain
    /// the response to match the schema. Others fall back to regular completion.
    async fn complete_with_schema(
        &self,
        messages: Vec<Message>,
        _schema: Option<Value>,
        max_tokens: Option<usize>,
    ) -> Result<String> {
        // Default: ignore schema, use regular completion
        self.complete_with_options(messages, max_tokens).await
    }
    /// Structured completion retaining usage and opaque conversation state.
    async fn complete_with_schema_response(
        &self,
        messages: Vec<Message>,
        schema: Option<Value>,
        max_tokens: Option<usize>,
    ) -> Result<ProviderResponse> {
        let content = self
            .complete_with_schema(messages, schema, max_tokens)
            .await?;
        Ok(ProviderResponse {
            content,
            ..Default::default()
        })
    }
}

impl std::fmt::Debug for ProviderResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderResponse")
            .field("content", &self.content)
            .field("reasoning_content", &self.reasoning_content)
            .field("tool_calls", &self.tool_calls)
            .field("finish_reason", &self.finish_reason)
            .field("inference_timing", &self.inference_timing)
            .field("quality_gate_retries", &self.quality_gate_retries)
            .field("provider_state", &self.provider_state)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;
    use serde_json::json;

    fn call(name: &str, id: Option<&str>) -> ParsedToolCall {
        ParsedToolCall {
            tool_name: name.to_string(),
            arguments: json!({}),
            call_id: id.map(str::to_string),
        }
    }

    #[spec("ASTRA-002")]
    #[test]
    fn chat_completions_tool_calls_replay_as_tool_role() {
        let response = ProviderResponse {
            tool_calls: vec![call("read", Some("call_abc"))],
            ..Default::default()
        };
        assert!(response.provider_state.is_empty());
        assert!(!response.has_native_response_calls());
        assert!(response.tool_results_use_tool_role());
        assert_eq!(
            response.as_assistant_message().tool_calls[0].id.as_deref(),
            Some("call_abc")
        );
    }

    #[spec("ASTRA-002")]
    #[test]
    fn responses_function_call_items_replay_as_tool_role() {
        let response = ProviderResponse {
            provider_state: ProviderState::openai_responses(vec![json!({
                "type": "function_call", "call_id": "fc_1", "name": "read", "arguments": "{}"
            })]),
            tool_calls: vec![call("read", Some("fc_1"))],
            ..Default::default()
        };
        assert!(response.tool_results_use_tool_role());
    }

    #[spec("ASTRA-002")]
    #[test]
    fn prose_extracted_calls_alongside_items_use_user_role() {
        let response = ProviderResponse {
            provider_state: ProviderState::openai_responses(vec![
                json!({"type": "reasoning", "id": "rs_1", "summary": []}),
                json!({"type": "message", "role": "assistant", "content": []}),
            ]),
            tool_calls: vec![call("read", None)],
            ..Default::default()
        };
        assert!(!response.tool_results_use_tool_role());
    }

    /// The assistant message must keep the tag, not just the items: without it
    /// the next request cannot tell whose wire format may replay them.
    #[spec("ASTRA-002")]
    #[test]
    fn assistant_message_carries_tagged_state_for_replay() {
        let response = ProviderResponse {
            provider_state: ProviderState::openai_responses(vec![json!({
                "type": "function_call", "call_id": "fc_1", "name": "read", "arguments": "{}"
            })]),
            tool_calls: vec![call("read", Some("fc_1"))],
            ..Default::default()
        };
        let state = response.as_assistant_message().provider_state;
        assert_eq!(state.native_call_ids().collect::<Vec<_>>(), vec!["fc_1"]);
        assert_eq!(
            state
                .replay_items_for(crate::provider_state::ProviderStateTag::OpenAiResponses)
                .map(|items| items.len()),
            Some(1)
        );
    }

    #[spec("ASTRA-002")]
    #[test]
    fn plain_text_turn_carries_no_tool_calls_to_pair() {
        let response = ProviderResponse {
            content: "hello".to_string(),
            ..Default::default()
        };
        assert!(response.tool_results_use_tool_role());
        assert!(response.as_assistant_message().tool_calls.is_empty());
    }
}
