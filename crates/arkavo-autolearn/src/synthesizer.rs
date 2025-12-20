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

/// Ministral-3B based policy synthesizer
///
/// When the `llm` feature is enabled, this uses arkavo-torg for
/// constrained decoding. Otherwise, it returns an error.
pub struct MinistralSynthesizer {
    config: SynthesizerConfig,
    #[cfg(feature = "llm")]
    _token_map: Option<()>, // Placeholder for arkavo-torg integration
}

impl MinistralSynthesizer {
    /// Create a new synthesizer with default config
    pub fn new() -> AutoLearnResult<Self> {
        Self::with_config(SynthesizerConfig::default())
    }

    /// Create a new synthesizer with custom config
    pub fn with_config(config: SynthesizerConfig) -> AutoLearnResult<Self> {
        Ok(Self {
            config,
            #[cfg(feature = "llm")]
            _token_map: None,
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
        let prompt = prompt.to_string();
        Box::pin(async move {
            // When the llm feature is enabled, use arkavo-torg
            #[cfg(feature = "llm")]
            {
                synthesize_with_torg(&prompt, &self.config).await
            }

            // When the llm feature is disabled, return an error
            #[cfg(not(feature = "llm"))]
            {
                let _ = prompt;
                Err(SynthesisError {
                    message: "LLM synthesis requires the 'llm' feature".to_string(),
                })
            }
        })
    }
}

/// Synthesize using arkavo-torg constrained decoding
#[cfg(feature = "llm")]
async fn synthesize_with_torg(
    _prompt: &str,
    _config: &SynthesizerConfig,
) -> Result<Graph, SynthesisError> {
    // TODO: Implement full arkavo-torg integration
    // This requires:
    // 1. Load Ministral-3B model via arkavo-llama-cpp
    // 2. Create MinistralTokenMap for vocabulary
    // 3. Use TorgLlamaSampler for constrained decoding
    // 4. Extract Graph from completed generation

    Err(SynthesisError {
        message: "arkavo-torg integration not yet implemented".to_string(),
    })
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
