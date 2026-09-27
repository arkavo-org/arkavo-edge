/// Budget-gate token estimator for ConversationWindow trimming.
/// Not a billing meter — trimming decisions tolerate 5-10% error
/// because the 70% budget has 30% headroom.
pub trait TokenEstimator: Send + Sync {
    fn estimate_tokens(&self, text: &str) -> usize;

    /// Counting-only fast path. Implementors can skip allocating
    /// a full token vector. Default: delegates to estimate_tokens.
    fn estimate_token_count(&self, text: &str) -> usize {
        self.estimate_tokens(text)
    }
}

/// Wraps an already-loaded llama.cpp model's tokenizer.
/// llama_tokenize() is a pure vocabulary lookup — no GPU, no inference.
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub(super) struct LlamaTokenEstimator {
    model: std::sync::Arc<arkavo_llm::LlamaModel>,
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
impl LlamaTokenEstimator {
    pub(super) fn new(model: std::sync::Arc<arkavo_llm::LlamaModel>) -> Self {
        Self { model }
    }
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
impl TokenEstimator for LlamaTokenEstimator {
    fn estimate_tokens(&self, text: &str) -> usize {
        self.estimate_token_count(text)
    }

    fn estimate_token_count(&self, text: &str) -> usize {
        if text.is_empty() {
            return 0;
        }
        let vocab = self.model.get_vocab();
        match arkavo_llm::tokenize_with_model(vocab, text.as_bytes()) {
            Ok(tokens) => tokens.len(),
            Err(_) => text.len() / 4, // fallback on tokenization error
        }
    }
}

/// Deterministic estimator for tests. Uses chars/4.
pub(super) struct MockEstimator;

impl TokenEstimator for MockEstimator {
    fn estimate_tokens(&self, text: &str) -> usize {
        self.estimate_token_count(text)
    }

    fn estimate_token_count(&self, text: &str) -> usize {
        if text.is_empty() {
            0
        } else {
            (text.len() / 4).max(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("SRV-010")]
    #[test]
    fn test_mock_estimator_empty_string() {
        let estimator = MockEstimator;
        assert_eq!(estimator.estimate_token_count(""), 0);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_mock_estimator_short_string() {
        let estimator = MockEstimator;
        assert_eq!(estimator.estimate_token_count("hi"), 1);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_mock_estimator_longer_string() {
        let estimator = MockEstimator;
        let text = "a".repeat(100);
        assert_eq!(estimator.estimate_token_count(&text), 25);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_mock_estimator_is_deterministic() {
        let estimator = MockEstimator;
        let text = "Hello, world! This is a test of token estimation.";
        let count1 = estimator.estimate_token_count(text);
        let count2 = estimator.estimate_token_count(text);
        assert_eq!(count1, count2);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_default_estimate_token_count_delegates() {
        let estimator = MockEstimator;
        let text = "test string for delegation";
        assert_eq!(
            estimator.estimate_tokens(text),
            estimator.estimate_token_count(text)
        );
    }
}
