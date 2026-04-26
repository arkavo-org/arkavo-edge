//! Token estimation and accounting helpers.
//!
//! Provides accurate token counts when the provider reports them via
//! `InferenceTiming`, and a more accurate fallback heuristic than the
//! historical `len() / 4` approximation.

use arkavo_llm::ProviderResponse;

/// Fallback characters-per-token ratio tuned for English + code mixtures.
///
/// Empirically closer to real tokenizer output than the legacy 4 chars/token
/// estimate, which systematically under-counts for code-heavy prompts.
const CHARS_PER_TOKEN: f32 = 3.6;

/// Estimate tokens for a text blob when no provider-reported count is
/// available.
///
/// Uses a word-aware heuristic: counts whitespace-separated tokens and
/// character-based tokens, takes the larger of the two. This is more robust
/// to whitespace-sparse code than pure character division.
pub fn estimate_tokens(text: &str) -> u32 {
    if text.is_empty() {
        return 0;
    }
    let char_estimate = (text.len() as f32 / CHARS_PER_TOKEN).ceil() as u32;
    let word_estimate = text
        .split_whitespace()
        .map(|w| 1 + (w.len() / 6) as u32)
        .sum::<u32>();
    char_estimate.max(word_estimate)
}

/// Extract accurate token usage from a `ProviderResponse` when the provider
/// reports it via `InferenceTiming`, otherwise fall back to estimation.
///
/// Returns `(input_tokens, output_tokens)`.
pub fn tokens_from_response(prompt: &str, response: &ProviderResponse) -> (u32, u32) {
    if let Some(timing) = &response.inference_timing {
        // Provider reported real counts.
        return (timing.n_prompt_eval, timing.n_eval);
    }
    // Fallback: estimate from text length.
    let input = estimate_tokens(prompt);
    let output = estimate_tokens(&response.content);
    (input, output)
}

/// Estimate token usage from a raw string response when no `ProviderResponse`
/// is available (e.g., `complete_with_schema` which returns `String`).
pub fn tokens_from_texts(prompt: &str, response: &str) -> (u32, u32) {
    (estimate_tokens(prompt), estimate_tokens(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_llm::provider::InferenceTiming;

    #[test]
    fn test_estimate_empty() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn test_estimate_nonzero() {
        let tokens = estimate_tokens("The quick brown fox jumps over the lazy dog");
        assert!(tokens > 0);
        assert!(tokens < 20, "got {tokens}");
    }

    #[test]
    fn test_estimate_code_heavy() {
        // Code with few spaces per token should not be under-counted.
        let code = "fn main() { let x = vec![1,2,3]; println!(\"{:?}\", x); }";
        let tokens = estimate_tokens(code);
        assert!(tokens >= (code.len() / 5) as u32);
    }

    #[test]
    fn test_tokens_from_response_uses_inference_timing() {
        let resp = ProviderResponse {
            content: "short".to_string(),
            reasoning_content: None,
            tool_calls: vec![],
            finish_reason: None,
            inference_timing: Some(InferenceTiming {
                prompt_eval_ms: 0.0,
                generation_ms: 0.0,
                n_prompt_eval: 123,
                n_eval: 45,
            }),
            quality_gate_retries: 0,
        };
        let (input, output) = tokens_from_response("long prompt text here", &resp);
        assert_eq!(input, 123, "should use provider-reported value");
        assert_eq!(output, 45);
    }

    #[test]
    fn test_tokens_from_response_falls_back() {
        let resp = ProviderResponse {
            content: "a response body".to_string(),
            reasoning_content: None,
            tool_calls: vec![],
            finish_reason: None,
            inference_timing: None,
            quality_gate_retries: 0,
        };
        let (input, output) = tokens_from_response("the prompt text", &resp);
        assert!(input > 0);
        assert!(output > 0);
    }

    #[test]
    fn test_tokens_from_texts() {
        let (i, o) = tokens_from_texts("prompt", "response");
        assert!(i > 0 && o > 0);
    }
}