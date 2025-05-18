// Model implementation - we now only use the Candle model
mod candle_model;

// Tokenizer implementations
mod tokenizer_hf;       // HuggingFace tokenizer
mod utils;

// Re-export everything
pub use candle_model::*;
pub use tokenizer_hf::*;
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
    
    // Candle model implementation - we only use this now
    model: Option<CandleQwen3Model>,
    
    // HuggingFace tokenizer
    hf_tokenizer: Option<tokenizer_hf::HfTokenizer>,
}

impl Qwen3Client {
    /// Creates a new Qwen3Client with the given configuration
    pub fn new(config: Qwen3Config) -> Self {
        Self {
            config,
            model: None,
            hf_tokenizer: None,
        }
    }
    
    /// Creates a new Qwen3Client using HuggingFace tokenizer explicitly
    pub fn new_with_hf_tokenizer(config: Qwen3Config) -> Self {
        Self {
            config,
            model: None,
            hf_tokenizer: None,
        }
    }

    /// Initializes the model and tokenizer
    pub async fn init(&mut self) -> Result<()> {
        // Initialize the Candle model
        #[cfg(feature = "embedded_model")]
        {
            self.model = Some(candle_model::CandleQwen3Model::new_from_embedded(&self.config)?);
        }
        
        #[cfg(not(feature = "embedded_model"))]
        {
            self.model = Some(candle_model::CandleQwen3Model::new(&self.config)?);
        }
        
        // Initialize HuggingFace tokenizer
        // Try possible locations for the tokenizer, from most specific to most general
        let possible_paths = [
            // Current directory
            "tokenizer.json",
            // Models subdirectory 
            "models/tokenizer.json",
            // Crate-specific path
            "crates/arkavo-llm/models/tokenizer.json",
        ];
        
        // Try each path until we find one that works
        let mut tokenizer_loaded = false;
        for path in possible_paths.iter() {
            if let Ok(tokenizer) = tokenizer_hf::HfTokenizer::new(path) {
                self.hf_tokenizer = Some(tokenizer);
                eprintln!("INFO: Using HuggingFace tokenizer from path: {}", path);
                tokenizer_loaded = true;
                break;
            }
        }
        
        // If nothing worked, try loading from the embedded data
        if !tokenizer_loaded {
            match tokenizer_hf::HfTokenizer::from_bytes(utils::EMBEDDED_TOKENIZER_JSON) {
                Ok(tokenizer) => {
                    self.hf_tokenizer = Some(tokenizer);
                    eprintln!("INFO: Using embedded HuggingFace tokenizer");
                }
                Err(err) => {
                    return Err(anyhow::anyhow!("Failed to load HuggingFace tokenizer: {}", err));
                }
            }
        }

        Ok(())
    }

    /// Generates text completion for the given prompt
    pub async fn generate(&self, prompt: &str) -> Result<String> {
        // Tokenize the prompt using HuggingFace tokenizer
        let tokenizer = self.hf_tokenizer
            .as_ref()
            .ok_or_else(|| LlmError::InferenceError("HuggingFace tokenizer not initialized".to_string()))?;
            
        let input_tokens = tokenizer.encode(prompt)?;
        
        // Generate response using Candle model
        let model = self.model
            .as_ref()
            .ok_or_else(|| LlmError::InferenceError("Model not initialized".to_string()))?;
            
        let output_tokens = model.generate(&input_tokens, self.config.max_tokens)?;
        
        // Decode response using HuggingFace tokenizer
        let raw_response = tokenizer.decode(&output_tokens)?;
        
        // Process and clean the response
        let clean_response = utils::extract_response(&raw_response);
        
        // Final sanity check - if output contains a lot of non-English characters, it's likely garbage
        let non_ascii_ratio = clean_response.chars()
            .filter(|c| !c.is_ascii() && !c.is_whitespace())
            .count() as f32 / clean_response.chars().count() as f32;
            
        if non_ascii_ratio > 0.3 {  // If more than 30% non-ASCII characters
            eprintln!("ERROR: Output contains too many non-ASCII characters ({}%). Model output appears corrupted.", 
                     non_ascii_ratio * 100.0);
            return Err(anyhow::anyhow!("Model output appears corrupted (contains {}% non-ASCII characters). This suggests a mismatch between the model and tokenizer.", 
                                     (non_ascii_ratio * 100.0) as i32));
        }

        Ok(clean_response)
    }

    /// Checks if the model is properly initialized
    pub async fn is_initialized(&self) -> bool {
        self.model.is_some() && self.hf_tokenizer.is_some()
    }
    
    /// Checks if the model is using GPU acceleration
    pub fn is_using_gpu(&self) -> bool {
        match &self.model {
            Some(model) => model.is_using_gpu(),
            None => false
        }
    }
    
    /// Returns the model implementation being used
    pub fn get_model_impl_name(&self) -> &'static str {
        "Candle (Accelerated)"
    }
    
    /// Returns the hardware acceleration being used
    pub fn get_acceleration_name(&self) -> &'static str {
        match &self.model {
            Some(model) => model.get_acceleration_name(),
            None => "CPU"
        }
    }
    
    /// Returns the tokenizer implementation being used
    pub fn get_tokenizer_impl_name(&self) -> &'static str {
        "HuggingFace Tokenizers"
    }
}