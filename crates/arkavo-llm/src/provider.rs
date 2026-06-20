use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_stream::Stream;

use crate::tool_parser::ParsedToolCall;
use crate::{Message, Result, StreamResponse};

/// Response from a provider that may include tool calls
#[derive(Debug, Clone, Default)]
pub struct ProviderResponse {
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

/// Timing and token-usage data from the LLM inference engine.
/// Populated by llama.cpp and by cloud providers that surface usage
/// metadata (Gemini 3.5's `usageMetadata` block in particular).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InferenceTiming {
    /// Prompt evaluation / prefill time in ms
    pub prompt_eval_ms: f64,
    /// Token generation time in ms
    pub generation_ms: f64,
    /// Number of prompt tokens evaluated (not served from KV cache)
    pub n_prompt_eval: u32,
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
            reasoning_content: None,
            tool_calls: Vec::new(),
            finish_reason: None,
            inference_timing: None,
            quality_gate_retries: 0,
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
}
