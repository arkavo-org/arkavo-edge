// Model implementation - we now only use the Candle model
mod candle_model;

// Tokenizer implementations
mod tokenizer_embedded; // Legacy embedded tokenizer (will be deprecated)
mod tokenizer_hf;       // HuggingFace tokenizer (preferred)
mod utils;

// Re-export everything
pub use candle_model::*;
pub use tokenizer_embedded::*; // Keep for backward compatibility
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
    
    // HuggingFace tokenizer (preferred)
    hf_tokenizer: Option<tokenizer_hf::HfTokenizer>,
    
    // Legacy embedded tokenizer (kept for backward compatibility)
    embedded_tokenizer: Option<tokenizer_embedded::EmbeddedQwen3Tokenizer>,
    
    // Whether to use the HF tokenizer (recommended)
    use_hf_tokenizer: bool,
}

impl Qwen3Client {
    /// Creates a new Qwen3Client with the given configuration
    pub fn new(config: Qwen3Config) -> Self {
        Self {
            config,
            model: None,
            hf_tokenizer: None,
            embedded_tokenizer: None,
            use_hf_tokenizer: true, // Default to using HuggingFace tokenizer
        }
    }
    
    /// Creates a new Qwen3Client using HuggingFace tokenizer explicitly
    pub fn new_with_hf_tokenizer(config: Qwen3Config) -> Self {
        Self {
            config,
            model: None,
            hf_tokenizer: None,
            embedded_tokenizer: None,
            use_hf_tokenizer: true, // Force HuggingFace tokenizer
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
        
        // Try to initialize HuggingFace tokenizer - always try first regardless of use_hf_tokenizer setting
        // Use absolute path for more reliable loading
        let tokenizer_path = std::path::Path::new("/Users/paul/Projects/arkavo/arkavo-edge/crates/arkavo-llm/models/tokenizer.json");
        let mut hf_tokenizer_loaded = false;
        
        if !tokenizer_path.exists() {
            eprintln!("WARN: Tokenizer file not found at '{}'", tokenizer_path.display());
            eprintln!("WARN: Looking for tokenizer.json in local directory");
            
            // Try with relative path
            match tokenizer_hf::HfTokenizer::new("./crates/arkavo-llm/models/tokenizer.json") {
                Ok(tokenizer) => {
                    self.hf_tokenizer = Some(tokenizer);
                    eprintln!("INFO: Using HuggingFace tokenizer from relative path");
                    hf_tokenizer_loaded = true;
                }
                Err(err) => {
                    eprintln!("WARN: Failed to initialize HuggingFace tokenizer from relative path: {}", err);
                }
            }
        } else {
            // Use absolute path if it exists
            match tokenizer_hf::HfTokenizer::new(tokenizer_path) {
                Ok(tokenizer) => {
                    self.hf_tokenizer = Some(tokenizer);
                    eprintln!("INFO: Using HuggingFace tokenizer from absolute path");
                    hf_tokenizer_loaded = true;
                }
                Err(err) => {
                    eprintln!("WARN: Failed to initialize HuggingFace tokenizer from absolute path: {}", err);
                }
            }
        }
        
        // If HF tokenizer couldn't be loaded but was specifically requested, return error
        if !hf_tokenizer_loaded && self.use_hf_tokenizer {
            return Err(anyhow::anyhow!("Failed to load HuggingFace tokenizer as explicitly requested"));
        }
        
        // Update use_hf_tokenizer flag based on successful load
        self.use_hf_tokenizer = hf_tokenizer_loaded;
        
        // Always initialize embedded tokenizer as fallback
        self.embedded_tokenizer = Some(tokenizer_embedded::EmbeddedQwen3Tokenizer::new()?);
        
        if !self.use_hf_tokenizer {
            eprintln!("INFO: Using embedded tokenizer");
        }

        Ok(())
    }

    /// Generates text completion for the given prompt
    pub async fn generate(&self, prompt: &str) -> Result<String> {
        eprintln!("DEBUG: Starting inference process...");
        
        // DEBUG: Print the exact prompt we're using
        eprintln!("DEBUG: ===== PROMPT =====");
        eprintln!("{}", prompt);
        eprintln!("DEBUG: =================");
        
        // Tokenize the prompt using the selected tokenizer
        let input_tokens = if self.use_hf_tokenizer {
            let tokenizer = self.hf_tokenizer
                .as_ref()
                .ok_or_else(|| LlmError::InferenceError("HuggingFace tokenizer not initialized".to_string()))?;
                
            tokenizer.encode(prompt)?
        } else {
            let tokenizer = self.embedded_tokenizer
                .as_ref()
                .ok_or_else(|| LlmError::InferenceError("Embedded tokenizer not initialized".to_string()))?;
                
            tokenizer.encode(prompt)?
        };
        
        eprintln!("DEBUG: First 30 token IDs: {:?}", &input_tokens.iter().take(30).collect::<Vec<_>>());
        
        // Perform a round-trip test to verify tokenizer
        let test_text = "Hello world";
        let test_tokens = if self.use_hf_tokenizer {
            self.hf_tokenizer.as_ref().unwrap().encode(test_text)?
        } else {
            self.embedded_tokenizer.as_ref().unwrap().encode(test_text)?
        };
        let decoded_test = if self.use_hf_tokenizer {
            self.hf_tokenizer.as_ref().unwrap().decode(&test_tokens)?
        } else {
            self.embedded_tokenizer.as_ref().unwrap().decode(&test_tokens)?
        };
        eprintln!("DEBUG: Tokenizer round-trip test: '{}' -> {} tokens -> '{}'", 
                 test_text, test_tokens.len(), decoded_test);

        // Generate response using Candle model
        eprintln!("DEBUG: Tokenization completed, starting model.generate()");
        
        let model = self.model
            .as_ref()
            .ok_or_else(|| LlmError::InferenceError("Model not initialized".to_string()))?;
            
        let output_tokens = model.generate(&input_tokens, self.config.max_tokens)?;
        
        eprintln!("DEBUG: Model.generate() completed successfully");
        eprintln!("DEBUG: Output tokens count: {}", output_tokens.len());
        eprintln!("DEBUG: First 20 output tokens: {:?}", &output_tokens.iter().take(20).collect::<Vec<_>>());
        eprintln!("DEBUG: Last 20 output tokens: {:?}", 
                 &output_tokens.iter().rev().take(20).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>());

        // Decode response using the selected tokenizer
        let raw_response = if self.use_hf_tokenizer {
            let tokenizer = self.hf_tokenizer
                .as_ref()
                .ok_or_else(|| LlmError::InferenceError("HuggingFace tokenizer not initialized".to_string()))?;
                
            tokenizer.decode(&output_tokens)?
        } else {
            let tokenizer = self.embedded_tokenizer
                .as_ref()
                .ok_or_else(|| LlmError::InferenceError("Embedded tokenizer not initialized".to_string()))?;
                
            tokenizer.decode(&output_tokens)?
        };
        
        // Collect first 100 Unicode characters (always safe)
        let preview: String = raw_response.chars().take(100).collect();
        eprintln!("DEBUG: First 100 chars of raw decoded output: {}{}", 
                 preview,
                 if raw_response.chars().count() > 100 { "..." } else { "" });
        
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
        let tokenizer_initialized = if self.use_hf_tokenizer {
            self.hf_tokenizer.is_some()
        } else {
            self.embedded_tokenizer.is_some()
        };
        
        self.model.is_some() && tokenizer_initialized
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
        if self.use_hf_tokenizer {
            "HuggingFace Tokenizers"
        } else {
            "Embedded (Legacy)"
        }
    }
}