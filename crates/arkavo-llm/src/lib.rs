// Model implementation - we now only use the Candle model
mod candle_model;
mod candle;

// Tokenizer implementations
mod tokenizer_hf;       // HuggingFace tokenizer (legacy - for non-GGUF models only)
mod tokenizer_gguf;     // GGUF built-in tokenizer (for GGUF models)
mod tokenizer_data;     // Generated tokenizer data (legacy - for non-GGUF models only)
mod embedded_model;     // Generated model data
mod utils;

// Re-export everything
pub use candle_model::*;
pub use tokenizer_hf::*;
pub use tokenizer_gguf::*;
pub use embedded_model::EMBEDDED_MODEL;
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
    
    // Tokenizer - we use GGUF tokenizer for GGUF models, HF tokenizer for safetensors models
    gguf_tokenizer: Option<tokenizer_gguf::GgufTokenizer>,
    hf_tokenizer: Option<tokenizer_hf::HfTokenizer>,
    
    // Flag to indicate which tokenizer we're using
    using_gguf_tokenizer: bool,
}

impl Qwen3Client {
    /// Creates a new Qwen3Client with the given configuration
    pub fn new(config: Qwen3Config) -> Self {
        Self {
            config,
            model: None,
            gguf_tokenizer: None,
            hf_tokenizer: None,
            using_gguf_tokenizer: true, // Default to GGUF tokenizer for embedded models
        }
    }
    
    /// Creates a new Qwen3Client using HuggingFace tokenizer explicitly
    /// This is only for non-GGUF models and should generally be avoided for GGUF models
    pub fn new_with_hf_tokenizer(config: Qwen3Config) -> Self {
        Self {
            config,
            model: None,
            gguf_tokenizer: None,
            hf_tokenizer: None,
            using_gguf_tokenizer: false,
        }
    }

    /// Initializes the model and tokenizer
    pub async fn init(&mut self) -> Result<()> {
        println!("Initializing Qwen3 model...");
        
        // Initialize the Candle model - always use embedded model
        // which is included directly via include_bytes!
        match candle_model::CandleQwen3Model::new_from_embedded(&self.config) {
            Ok(model) => {
                println!("Successfully loaded embedded GGUF model");
                self.model = Some(model);
            },
            Err(err) => {
                println!("Error loading model: {}", err);
                return Err(anyhow::anyhow!("Failed to load embedded Qwen3 model: {}", err));
            }
        }
        
        // For GGUF models, extract the tokenizer data directly from the GGUF file
        if self.using_gguf_tokenizer {
            println!("Initializing GGUF tokenizer from embedded model data...");
            match tokenizer_gguf::GgufTokenizer::new(crate::EMBEDDED_MODEL) {
                Ok(tokenizer) => {
                    println!("Successfully initialized GGUF tokenizer with {} tokens", tokenizer.vocab_size());
                    
                    // Print basic vocabulary information
                    println!("GGUF tokenizer initialized with {} tokens", tokenizer.vocab_size());
                    
                    // Do a simple verification to make sure basic tokens are present
                    let basic_token_test = [
                        (" ", 259),     // Space token ID
                        ("\n", 285),    // Newline token ID
                        ("<|im_start|>", 151643), // Common special token
                    ];
                    
                    // Quick verification for critical tokens to validate the tokenizer
                    let mut missing_tokens = Vec::new();
                    for (token_str, expected_id) in &basic_token_test {
                        match tokenizer.encode(token_str) {
                            Ok(ids) if ids.contains(expected_id) => {
                                println!("✓ Verified token: {} (found ID: {})", token_str, expected_id);
                            },
                            Ok(ids) => {
                                // This is a "soft" failure - we found a token, but not with the expected ID
                                println!("! Basic token found with different ID: {} expected ID {}, got {:?}", 
                                        token_str, expected_id, ids);
                                // Don't consider this missing - the token can be encoded
                            },
                            Err(_) => {
                                println!("! Error encoding basic token: {}", token_str);
                                missing_tokens.push(*token_str);
                            }
                        }
                    }
                    
                    // If critical tokens are missing, show a more detailed warning
                    if !missing_tokens.is_empty() {
                        println!("\n⚠️ WARNING: Basic token verification failed for: {:?}", missing_tokens);
                        println!("The tokenizer may not produce optimal results. Consider running tests to diagnose.");
                    } else {
                        println!("Basic tokenizer verification passed. Tokenizer is ready for use.");
                    }
                    
                    self.gguf_tokenizer = Some(tokenizer);
                },
                Err(err) => {
                    println!("Error initializing GGUF tokenizer: {}", err);
                    println!("Falling back to HuggingFace tokenizer...");
                    self.using_gguf_tokenizer = false;
                }
            }
        }
        
        // Fall back to HuggingFace tokenizer if GGUF tokenizer initialization failed
        // or if HuggingFace tokenizer was explicitly requested
        if !self.using_gguf_tokenizer {
            println!("Loading HuggingFace tokenizer...");
            match tokenizer_hf::HfTokenizer::from_bytes(utils::EMBEDDED_TOKENIZER_JSON) {
                Ok(tokenizer) => {
                    println!("Successfully loaded embedded HuggingFace tokenizer");
                    self.hf_tokenizer = Some(tokenizer);
                }
                Err(err) => {
                    println!("Error loading HuggingFace tokenizer: {}", err);
                    return Err(anyhow::anyhow!("Failed to load any tokenizer: {}", err));
                }
            }
        }

        Ok(())
    }

    /// Generates text completion for the given prompt
    pub async fn generate(&self, prompt: &str) -> Result<String> {
        // Format the prompt using Qwen3's expected chat template
        let formatted_prompt = self.format_qwen3_prompt(prompt);
        println!("Formatted prompt for Qwen3:");
        println!("---BEGIN PROMPT---");
        println!("{}", formatted_prompt);
        println!("---END PROMPT---");
        
        // Use the appropriate tokenizer based on what was initialized
        let input_tokens = if self.using_gguf_tokenizer {
            // Use GGUF tokenizer
            let tokenizer = self.gguf_tokenizer
                .as_ref()
                .ok_or_else(|| LlmError::InferenceError("GGUF tokenizer not initialized".to_string()))?;
                
            let tokens = tokenizer.encode(&formatted_prompt)?;
            
            // Validate that our tokenization doesn't have UNK tokens (id 0)
            let unk_count = tokens.iter().filter(|&&id| id == 0).count();
            if unk_count > 0 {
                println!("⚠️ Warning: Input contains {} unknown tokens (<unk>) out of {} ({}%)", 
                         unk_count, tokens.len(), 
                         (unk_count as f32 / tokens.len() as f32) * 100.0);
            }
            
            tokens
        } else {
            // Fall back to HuggingFace tokenizer
            let tokenizer = self.hf_tokenizer
                .as_ref()
                .ok_or_else(|| LlmError::InferenceError("HuggingFace tokenizer not initialized".to_string()))?;
                
            tokenizer.encode(&formatted_prompt)?
        };
        
        println!("Input tokens: {} tokens", input_tokens.len());
        
        // Generate response using Candle model
        let model = self.model
            .as_ref()
            .ok_or_else(|| LlmError::InferenceError("Model not initialized".to_string()))?;
        
        // Check that we have input tokens
        if input_tokens.is_empty() {
            return Err(anyhow::anyhow!("No input tokens generated - check tokenizer configuration"));
        }
            
        let output_tokens = model.generate(&input_tokens, self.config.max_tokens)?;
        
        println!("Generated tokens: {} tokens", output_tokens.len());
        
        // Debug token IDs
        if !output_tokens.is_empty() {
            // Print first few token IDs for debugging
            let max_debug_tokens = std::cmp::min(output_tokens.len(), 10);
            println!("First {} token IDs: {:?}", max_debug_tokens, &output_tokens[..max_debug_tokens]);
            
            // Print tail tokens if longer than 10
            if output_tokens.len() > 10 {
                let tail_start = std::cmp::max(output_tokens.len() - 5, 10);
                println!("Last 5 token IDs: {:?}", &output_tokens[tail_start..]);
            }
            
            // Validate output token quality
            let unk_count = output_tokens.iter().filter(|&&id| id == 0).count();
            if unk_count > 0 {
                println!("⚠️ Warning: Output contains {} unknown tokens (<unk>) out of {} ({}%)", 
                         unk_count, output_tokens.len(), 
                         (unk_count as f32 / output_tokens.len() as f32) * 100.0);
            }
        }
        
        // Decode response using the same tokenizer used for encoding
        println!("Decoding tokens to text...");
        let raw_response = if self.using_gguf_tokenizer {
            // Use GGUF tokenizer
            let tokenizer = self.gguf_tokenizer
                .as_ref()
                .ok_or_else(|| LlmError::InferenceError("GGUF tokenizer not initialized".to_string()))?;
            
            let decoded = tokenizer.decode(&output_tokens)?;
            println!("Successfully decoded with GGUF tokenizer");
            decoded
        } else {
            // Fall back to HuggingFace tokenizer
            let tokenizer = self.hf_tokenizer
                .as_ref()
                .ok_or_else(|| LlmError::InferenceError("HuggingFace tokenizer not initialized".to_string()))?;
            
            let decoded = tokenizer.decode(&output_tokens)?;
            println!("Successfully decoded with HF tokenizer");
            decoded
        };
        
        // Process and clean the response
        let clean_response = utils::extract_response(&raw_response);
        
        // Final sanity check - if output contains a lot of non-English characters, it's likely garbage
        let non_ascii_ratio = clean_response.chars()
            .filter(|c| !c.is_ascii() && !c.is_whitespace())
            .count() as f32 / clean_response.chars().count().max(1) as f32; // Avoid division by zero
            
        if non_ascii_ratio > 0.3 && clean_response.chars().count() > 5 {  // Only check if response is longer than 5 chars
            return Err(anyhow::anyhow!("Model output appears corrupted (contains {}% non-ASCII characters). This suggests a mismatch between the model and tokenizer.", 
                                     (non_ascii_ratio * 100.0) as i32));
        }

        Ok(clean_response)
    }

    /// Checks if the model is properly initialized
    pub async fn is_initialized(&self) -> bool {
        self.model.is_some() && 
        (self.gguf_tokenizer.is_some() || self.hf_tokenizer.is_some())
    }
    
    /// Gets the tokenizer type currently in use
    pub fn get_tokenizer_impl_name(&self) -> &'static str {
        if self.using_gguf_tokenizer {
            "GGUF Built-in Tokenizer"
        } else {
            "HuggingFace Tokenizers"
        }
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
    
    /// Format the prompt according to Qwen3's expected chat template
    fn format_qwen3_prompt(&self, user_prompt: &str) -> String {
        // For simple prompts, we'll use Qwen3's ChatML format
        // This follows the structure:
        // <|im_start|>system
        // [system prompt]<|im_end|>
        // <|im_start|>user
        // [user message]<|im_end|>
        // <|im_start|>assistant
        
        // Default system prompt
        let system_prompt = "You are Qwen, a helpful, respectful and honest AI assistant.";
        
        // Build the formatted prompt
        let mut formatted = String::new();
        
        // Add system message
        formatted.push_str("<|im_start|>system\n");
        formatted.push_str(system_prompt);
        formatted.push_str("<|im_end|>\n");
        
        // Add user message
        formatted.push_str("<|im_start|>user\n");
        formatted.push_str(user_prompt);
        formatted.push_str("<|im_end|>\n");
        
        // Add assistant prefix (model will continue from here)
        formatted.push_str("<|im_start|>assistant\n");
        
        formatted
    }
}