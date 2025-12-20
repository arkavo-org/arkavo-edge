//! LLM-based policy synthesis using Ministral-3B
//!
//! Implements the `LlmSynthesizer` trait from arkavo-ensemble using
//! Ministral-3B with constrained decoding via arkavo-torg.

use std::future::Future;
use std::pin::Pin;

use torg_core::Graph;

use arkavo_ensemble::{LlmSynthesizer, SynthesisError};

use crate::error::AutoLearnResult;
use crate::signals::PainSignal;

#[cfg(feature = "llm")]
use std::path::Path;
#[cfg(feature = "llm")]
use crate::error::AutoLearnError;
#[cfg(feature = "llm")]
use arkavo_llama_cpp::{
    LlamaContext, LlamaModel, LlamaSampler, batch_free, batch_init_with_tokens, decode_batch,
    detect_model_format, tokenize_with_model,
};
#[cfg(feature = "llm")]
use arkavo_torg::{
    MinistralTokenMap, Qwen3TokenMap, TorgLlamaSampler, TokenMapping, format_prompt,
};

/// Configuration for the synthesizer
#[derive(Debug, Clone)]
pub struct SynthesizerConfig {
    /// Model identifier
    pub model_id: String,
    /// Maximum tokens to generate
    pub max_tokens: usize,
    /// Temperature for sampling (0.0 = greedy)
    pub temperature: f32,
}

impl Default for SynthesizerConfig {
    fn default() -> Self {
        Self {
            model_id: "ministral-3b".to_string(),
            max_tokens: 150,
            temperature: 0.0, // Deterministic for reproducibility
        }
    }
}

/// Token map variant for different model families
#[cfg(feature = "llm")]
enum TokenMapVariant {
    Ministral { mapping: TokenMapping, vocab_size: i32 },
    Qwen3 { mapping: TokenMapping, vocab_size: i32 },
}

/// Ministral-3B based policy synthesizer
///
/// When the `llm` feature is enabled, this uses arkavo-torg for
/// constrained decoding. Otherwise, it returns an error.
pub struct MinistralSynthesizer {
    config: SynthesizerConfig,
    #[cfg(feature = "llm")]
    model: Option<LlamaModel>,
    #[cfg(feature = "llm")]
    token_map: Option<TokenMapVariant>,
}

impl MinistralSynthesizer {
    /// Create a new synthesizer with default config (no model loaded)
    ///
    /// Without loading a model, synthesis will return an error.
    /// Use `with_model()` to create a synthesizer with LLM capabilities.
    pub fn new() -> AutoLearnResult<Self> {
        Self::with_config(SynthesizerConfig::default())
    }

    /// Create a new synthesizer with custom config (no model loaded)
    pub fn with_config(config: SynthesizerConfig) -> AutoLearnResult<Self> {
        Ok(Self {
            config,
            #[cfg(feature = "llm")]
            model: None,
            #[cfg(feature = "llm")]
            token_map: None,
        })
    }

    /// Create a synthesizer with a loaded model
    ///
    /// # Arguments
    ///
    /// * `config` - Synthesizer configuration
    /// * `model_path` - Path to the GGUF model file
    ///
    /// # Errors
    ///
    /// Returns an error if the model cannot be loaded or token mapping fails.
    #[cfg(feature = "llm")]
    pub fn with_model(config: SynthesizerConfig, model_path: &Path) -> Result<Self, AutoLearnError> {
        let model = LlamaModel::from_file(model_path.to_str().ok_or_else(|| {
            AutoLearnError::Llm("Invalid model path".to_string())
        })?).map_err(|e| AutoLearnError::Llm(e))?;

        let vocab = model.get_vocab();

        // Auto-detect model type from model_id or path
        let model_name = model_path.to_string_lossy().to_lowercase();
        let token_map = if model_name.contains("qwen") || config.model_id.to_lowercase().contains("qwen") {
            // SAFETY: vocab pointer is valid from model
            let map = unsafe { Qwen3TokenMap::from_vocab(vocab) }
                .map_err(|e| AutoLearnError::Llm(format!("Qwen3 token map error: {e}")))?;
            let vocab_size = map.vocab_size();
            TokenMapVariant::Qwen3 {
                mapping: map.into_mapping(),
                vocab_size,
            }
        } else {
            // Default to Ministral
            // SAFETY: vocab pointer is valid from model
            let map = unsafe { MinistralTokenMap::from_vocab(vocab) }
                .map_err(|e| AutoLearnError::Llm(format!("Ministral token map error: {e}")))?;
            let vocab_size = map.vocab_size();
            TokenMapVariant::Ministral {
                mapping: map.into_mapping(),
                vocab_size,
            }
        };

        tracing::info!(
            "Loaded model from {} with {} token mapping",
            model_path.display(),
            match &token_map {
                TokenMapVariant::Ministral { .. } => "Ministral",
                TokenMapVariant::Qwen3 { .. } => "Qwen3",
            }
        );

        Ok(Self {
            config,
            model: Some(model),
            token_map: Some(token_map),
        })
    }

    /// Get the model ID
    #[must_use]
    pub fn model_id(&self) -> &str {
        &self.config.model_id
    }

    /// Synthesize a patchlet from a pain signal
    pub async fn synthesize_patchlet(&self, signal: &PainSignal) -> Result<Graph, SynthesisError> {
        let prompt = signal.context.build_prompt();
        self.synthesize(&prompt).await
    }

    /// Build a synthesis prompt from a pain signal
    #[must_use]
    pub fn build_prompt(&self, signal: &PainSignal) -> String {
        signal.context.build_prompt()
    }
}

impl Default for MinistralSynthesizer {
    fn default() -> Self {
        Self::new().expect("Failed to create default synthesizer")
    }
}

impl LlmSynthesizer for MinistralSynthesizer {
    fn synthesize(
        &self,
        prompt: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Graph, SynthesisError>> + Send + '_>> {
        // When the llm feature is enabled, use arkavo-torg with the loaded model
        #[cfg(feature = "llm")]
        {
            // Check if model is loaded
            let (model, token_map) = match (&self.model, &self.token_map) {
                (Some(m), Some(t)) => (m, t),
                _ => {
                    return Box::pin(async {
                        Err(SynthesisError {
                            message: "Model not loaded. Call with_model() to load an LLM.".to_string(),
                        })
                    });
                }
            };

            // Clone config for the async block
            let config = self.config.clone();
            let prompt = prompt.to_string();

            // Note: synthesize_with_torg is synchronous (blocking LLM inference)
            // We wrap it in spawn_blocking to avoid blocking the async runtime
            Box::pin(async move {
                // For now, run synchronously since llama.cpp context is not Send
                // TODO: Consider spawn_blocking if this becomes a bottleneck
                synthesize_with_torg(model, token_map, &prompt, &config)
            })
        }

        // When the llm feature is disabled, return an error
        #[cfg(not(feature = "llm"))]
        {
            let _ = prompt;
            Box::pin(async {
                Err(SynthesisError {
                    message: "LLM synthesis requires the 'llm' feature".to_string(),
                })
            })
        }
    }
}

/// Synthesize using arkavo-torg constrained decoding
#[cfg(feature = "llm")]
fn synthesize_with_torg(
    model: &LlamaModel,
    token_map: &TokenMapVariant,
    prompt: &str,
    config: &SynthesizerConfig,
) -> Result<Graph, SynthesisError> {
    // 1. Create fresh context for this generation
    let ctx = LlamaContext::new(model)
        .map_err(|e| SynthesisError { message: e.to_string() })?;

    // Get mapping and vocab size
    let (mapping, vocab_size) = match token_map {
        TokenMapVariant::Ministral { mapping, vocab_size } => (mapping.clone(), *vocab_size),
        TokenMapVariant::Qwen3 { mapping, vocab_size } => (mapping.clone(), *vocab_size),
    };

    let mut torg_sampler = TorgLlamaSampler::new(mapping, vocab_size);

    // 2. Detect format and build prompt with chat template
    let format = detect_model_format(&config.model_id);
    let formatted = format_prompt(prompt, format);

    tracing::debug!("Formatted prompt ({} chars): {}...",
        formatted.len(),
        &formatted[..formatted.len().min(100)]
    );

    // 3. Tokenize and process prompt
    let vocab = model.get_vocab();
    let tokens = tokenize_with_model(vocab, formatted.as_bytes())
        .map_err(|e| SynthesisError { message: e })?;

    let mut batch = batch_init_with_tokens(&tokens, 0, true);
    decode_batch(&ctx, batch).map_err(|e| SynthesisError { message: e })?;
    batch_free(&mut batch);

    let mut pos = tokens.len() as i32;
    let eos = model.get_eos_token();

    // 4. Generation loop with constraints (CRITICAL ORDER)
    for iteration in 0..config.max_tokens {
        // Get TØR-G constraints FIRST
        let bias = torg_sampler.get_logit_bias();

        // Create sampler chain: logit_bias → temp → greedy/top_p
        let sampler = LlamaSampler::new_chain(true)
            .map_err(|e| SynthesisError { message: e })?;
        sampler.add_logit_bias(torg_sampler.vocab_size(), &bias);
        sampler.add_temp(config.temperature);
        if config.temperature == 0.0 {
            sampler.add_greedy();
        } else {
            sampler.add_top_p(0.95, 1);  // min_keep = 1
        }

        let token = sampler.sample(&ctx, -1);

        // Check for completion
        if token == eos || torg_sampler.is_complete() {
            tracing::debug!("Generation complete at iteration {}", iteration);
            break;
        }

        // Feed to state machine
        if let Err(e) = torg_sampler.feed_token(token as u32) {
            tracing::warn!("TØR-G feed error at iteration {}: {:?}", iteration, e);
            return Err(SynthesisError {
                message: format!("Constrained decoding error: {e}"),
            });
        }

        // Advance context
        let mut next_batch = batch_init_with_tokens(&[token], pos, true);
        decode_batch(&ctx, next_batch).map_err(|e| SynthesisError { message: e })?;
        batch_free(&mut next_batch);
        pos += 1;
    }

    // 5. Extract graph
    if !torg_sampler.is_complete() {
        return Err(SynthesisError {
            message: format!("Max tokens ({}) reached without completing graph", config.max_tokens),
        });
    }

    torg_sampler
        .finish()
        .map_err(|e| SynthesisError { message: e.to_string() })
}

/// Mock synthesizer for testing
#[cfg(test)]
pub struct MockSynthesizer {
    graph: Graph,
}

#[cfg(test)]
impl MockSynthesizer {
    pub fn new(graph: Graph) -> Self {
        Self { graph }
    }
}

#[cfg(test)]
impl LlmSynthesizer for MockSynthesizer {
    fn synthesize(
        &self,
        _prompt: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Graph, SynthesisError>> + Send + '_>> {
        let graph = self.graph.clone();
        Box::pin(async move { Ok(graph) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use torg_core::{Builder, Token};

    fn create_simple_graph() -> Graph {
        let mut builder = Builder::new();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::Or).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::NodeEnd).unwrap();
        builder.push(Token::OutputDecl).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.finish().unwrap()
    }

    #[test]
    fn test_synthesizer_config_default() {
        let config = SynthesizerConfig::default();
        assert_eq!(config.model_id, "ministral-3b");
        assert_eq!(config.temperature, 0.0);
    }

    #[test]
    fn test_synthesizer_creation() {
        let synthesizer = MinistralSynthesizer::new().unwrap();
        assert_eq!(synthesizer.model_id(), "ministral-3b");
    }

    #[tokio::test]
    async fn test_mock_synthesizer() {
        let graph = create_simple_graph();
        let mock = MockSynthesizer::new(graph.clone());

        let result = mock.synthesize("test prompt").await.unwrap();
        assert_eq!(result.inputs, graph.inputs);
        assert_eq!(result.outputs, graph.outputs);
    }

    #[test]
    fn test_build_prompt() {
        let _graph = create_simple_graph();
        let ctx = arkavo_ensemble::SynthesisContext::new(
            "test-model".to_string(),
            "Allow if admin".to_string(),
        )
        .with_inputs(vec![0, 1])
        .with_outputs(vec![2]);

        let signal = crate::signals::PainSignal::new(
            crate::signals::PainSource::External {
                description: "test".to_string(),
            },
            0.5,
            ctx,
        );

        let synthesizer = MinistralSynthesizer::new().unwrap();
        let prompt = synthesizer.build_prompt(&signal);

        assert!(prompt.contains("Allow if admin"));
        assert!(prompt.contains("Inputs:"));
    }
}
