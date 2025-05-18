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
        // Use absolute path for more reliable loading
        let tokenizer_path = std::path::Path::new("/Users/paul/Projects/arkavo/arkavo-edge/crates/arkavo-llm/models/tokenizer.json");
        
        if !tokenizer_path.exists() {
            eprintln!("WARN: Tokenizer file not found at '{}'", tokenizer_path.display());
            eprintln!("WARN: Looking for tokenizer.json in local directory");
            
            // Try with relative path
            match tokenizer_hf::HfTokenizer::new("./crates/arkavo-llm/models/tokenizer.json") {
                Ok(tokenizer) => {
                    self.hf_tokenizer = Some(tokenizer);
                    eprintln!("INFO: Using HuggingFace tokenizer from relative path");
                }
                Err(err) => {
                    return Err(anyhow::anyhow!("Failed to load HuggingFace tokenizer: {}", err));
                }
            }
        } else {
            // Use absolute path if it exists
            match tokenizer_hf::HfTokenizer::new(tokenizer_path) {
                Ok(tokenizer) => {
                    self.hf_tokenizer = Some(tokenizer);
                    eprintln!("INFO: Using HuggingFace tokenizer from absolute path");
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
        eprintln!("DEBUG: Starting inference process...");
        
        // DEBUG: Print the exact prompt we're using
        eprintln!("DEBUG: ===== PROMPT =====");
        eprintln!("{}", prompt);
        eprintln!("DEBUG: =================");
        
        // Tokenize the prompt using HuggingFace tokenizer
        let tokenizer = self.hf_tokenizer
            .as_ref()
            .ok_or_else(|| LlmError::InferenceError("HuggingFace tokenizer not initialized".to_string()))?;
            
        let input_tokens = tokenizer.encode(prompt)?;
        
        eprintln!("DEBUG: First 30 token IDs: {:?}", &input_tokens.iter().take(30).collect::<Vec<_>>());
        
        // Perform a round-trip test to verify tokenizer
        let test_text = "Hello world";
        let test_tokens = tokenizer.encode(test_text)?;
        let decoded_test = tokenizer.decode(&test_tokens)?;
        
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

        // Decode response using HuggingFace tokenizer
        let raw_response = tokenizer.decode(&output_tokens)?;
        
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