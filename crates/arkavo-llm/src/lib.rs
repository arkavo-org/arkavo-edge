mod model;
mod tokenizer;
mod utils;

pub use model::*;
pub use tokenizer::*;
pub use utils::*;

use anyhow::Result;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LlmError {
    #[error("Failed to load model: {0}")]
    ModelLoadError(String),

    #[error("Failed to tokenize input: {0}")]
    TokenizationError(String),

    #[error("Inference error: {0}")]
    InferenceError(String),
}

/// Configuration for Qwen3 model
#[derive(Debug, Clone)]
pub struct Qwen3Config {
    /// Path to the model files (virtual path for embedded model)
    pub model_path: String,

    /// Temperature for text generation (0.0-1.0)
    pub temperature: f32,

    /// Whether to use GPU for inference
    pub use_gpu: bool,

    /// Maximum tokens to generate
    pub max_tokens: usize,
}

impl Default for Qwen3Config {
    fn default() -> Self {
        Self {
            model_path: String::from("memory://qwen3-0.6b"),
            temperature: 0.7,
            use_gpu: false,
            max_tokens: 1024,
        }
    }
}

/// Main interface for interacting with Qwen3 model
pub struct Qwen3Client {
    config: Qwen3Config,
    model: Option<model::Qwen3Model>,
    tokenizer: Option<tokenizer::Qwen3Tokenizer>,
}

impl Qwen3Client {
    /// Creates a new Qwen3Client with the given configuration
    pub fn new(config: Qwen3Config) -> Self {
        Self {
            config,
            model: None,
            tokenizer: None,
        }
    }

    /// Initializes the model and tokenizer
    pub async fn init(&mut self) -> Result<()> {
        // Initialize tokenizer with embedded model data
        self.tokenizer = Some(tokenizer::Qwen3Tokenizer::new_from_embedded()?);
        self.model = Some(model::Qwen3Model::new_from_embedded(&self.config)?);

        Ok(())
    }

    /// Generates text completion for the given prompt
    pub async fn generate(&self, prompt: &str) -> Result<String> {
        // Validate model and tokenizer are initialized
        let model = self
            .model
            .as_ref()
            .ok_or_else(|| LlmError::InferenceError("Model not initialized".to_string()))?;

        let tokenizer = self
            .tokenizer
            .as_ref()
            .ok_or_else(|| LlmError::InferenceError("Tokenizer not initialized".to_string()))?;

        // Tokenize the prompt
        let input_tokens = tokenizer.encode(prompt)?;

        // Generate response
        let output_tokens = model.generate(&input_tokens, self.config.max_tokens)?;

        // Decode response
        let raw_response = tokenizer.decode(&output_tokens)?;
        
        // Process and clean the response
        let clean_response = utils::extract_response(&raw_response);

        Ok(clean_response)
    }

    /// Checks if the model is properly initialized
    pub async fn is_initialized(&self) -> bool {
        self.model.is_some() && self.tokenizer.is_some()
    }
}