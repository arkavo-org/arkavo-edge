mod model;
mod tokenizer;
mod tokenizer_static;
mod tokenizer_embedded;
mod utils;

pub use model::*;
pub use tokenizer::*;
pub use tokenizer_static::*;
pub use tokenizer_embedded::*;
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
    
    // Different tokenizer implementations
    #[allow(dead_code)]
    tokenizer: Option<tokenizer::Qwen3Tokenizer>,
    #[allow(dead_code)]
    static_tokenizer: Option<tokenizer_static::StaticQwen3Tokenizer>,
    embedded_tokenizer: Option<tokenizer_embedded::EmbeddedQwen3Tokenizer>,
    
    // Which tokenizer to use (embedded is most reliable)
    tokenizer_type: TokenizerType,
}

/// Available tokenizer implementations
enum TokenizerType {
    /// Dynamic tokenizer that parses tokenizer.json at runtime
    Dynamic,
    /// Static tokenizer that was processed at build time
    Static,
    /// Embedded tokenizer with hardcoded common tokens
    Embedded,
}

impl Qwen3Client {
    /// Creates a new Qwen3Client with the given configuration
    pub fn new(config: Qwen3Config) -> Self {
        Self {
            config,
            model: None,
            tokenizer: None,
            static_tokenizer: None,
            embedded_tokenizer: None,
            tokenizer_type: TokenizerType::Embedded, // Use embedded by default - most reliable
        }
    }

    /// Initializes the model and tokenizer
    pub async fn init(&mut self) -> Result<()> {
        // Initialize model (using regular new method, which works with or without the embedded_model feature)
        self.model = Some(model::Qwen3Model::new(&self.config)?);
        
        // Always initialize the embedded tokenizer first (most reliable)
        self.embedded_tokenizer = Some(tokenizer_embedded::EmbeddedQwen3Tokenizer::new()?);
        
        // Try other tokenizers as fallbacks based on preference
        match self.tokenizer_type {
            TokenizerType::Static => {
                match tokenizer_static::StaticQwen3Tokenizer::new() {
                    Ok(static_tokenizer) => {
                        self.static_tokenizer = Some(static_tokenizer);
                        self.tokenizer_type = TokenizerType::Static;
                    },
                    Err(_) => {
                        // Fall back to embedded
                        self.tokenizer_type = TokenizerType::Embedded;
                    }
                }
            },
            TokenizerType::Dynamic => {
                match tokenizer::Qwen3Tokenizer::new_from_embedded() {
                    Ok(dynamic_tokenizer) => {
                        self.tokenizer = Some(dynamic_tokenizer);
                        self.tokenizer_type = TokenizerType::Dynamic;
                    },
                    Err(_) => {
                        // Fall back to embedded
                        self.tokenizer_type = TokenizerType::Embedded;
                    }
                }
            },
            TokenizerType::Embedded => {
                // Already initialized
            }
        }

        Ok(())
    }

    /// Generates text completion for the given prompt
    pub async fn generate(&self, prompt: &str) -> Result<String> {
        // Validate model is initialized
        let model = self
            .model
            .as_ref()
            .ok_or_else(|| LlmError::InferenceError("Model not initialized".to_string()))?;

        // Tokenize the prompt using the appropriate tokenizer
        let input_tokens = match self.tokenizer_type {
            TokenizerType::Static => {
                if let Some(tokenizer) = &self.static_tokenizer {
                    tokenizer.encode(prompt)?
                } else {
                    // Fall back to embedded
                    self.embedded_tokenizer
                        .as_ref()
                        .ok_or_else(|| LlmError::InferenceError("No tokenizer available".to_string()))?
                        .encode(prompt)?
                }
            },
            TokenizerType::Dynamic => {
                if let Some(tokenizer) = &self.tokenizer {
                    tokenizer.encode(prompt)?
                } else {
                    // Fall back to embedded
                    self.embedded_tokenizer
                        .as_ref()
                        .ok_or_else(|| LlmError::InferenceError("No tokenizer available".to_string()))?
                        .encode(prompt)?
                }
            },
            TokenizerType::Embedded => {
                self.embedded_tokenizer
                    .as_ref()
                    .ok_or_else(|| LlmError::InferenceError("Embedded tokenizer not initialized".to_string()))?
                    .encode(prompt)?
            }
        };

        // Generate response
        let output_tokens = model.generate(&input_tokens, self.config.max_tokens)?;

        // Decode response using the appropriate tokenizer
        let raw_response = match self.tokenizer_type {
            TokenizerType::Static => {
                if let Some(tokenizer) = &self.static_tokenizer {
                    tokenizer.decode(&output_tokens)?
                } else {
                    // Fall back to embedded
                    self.embedded_tokenizer
                        .as_ref()
                        .ok_or_else(|| LlmError::InferenceError("No tokenizer available".to_string()))?
                        .decode(&output_tokens)?
                }
            },
            TokenizerType::Dynamic => {
                if let Some(tokenizer) = &self.tokenizer {
                    tokenizer.decode(&output_tokens)?
                } else {
                    // Fall back to embedded
                    self.embedded_tokenizer
                        .as_ref()
                        .ok_or_else(|| LlmError::InferenceError("No tokenizer available".to_string()))?
                        .decode(&output_tokens)?
                }
            },
            TokenizerType::Embedded => {
                self.embedded_tokenizer
                    .as_ref()
                    .ok_or_else(|| LlmError::InferenceError("Embedded tokenizer not initialized".to_string()))?
                    .decode(&output_tokens)?
            }
        };
        
        // Process and clean the response
        let clean_response = utils::extract_response(&raw_response);

        Ok(clean_response)
    }

    /// Checks if the model is properly initialized
    pub async fn is_initialized(&self) -> bool {
        self.model.is_some() && self.embedded_tokenizer.is_some()
    }
}