// Model implementation - we now only use the Candle model
mod candle_model_core;     // Core model structure and basic methods
mod candle_model_create;   // Model creation methods
mod candle_model_gguf;     // GGUF model loading functionality
mod candle_transformer_layer; // Transformer layer definition
mod candle_forward;        // Forward pass implementation 
mod candle_generation;     // Token generation logic
mod candle_kv_cache;       // Key-value cache for efficient generation

// Tokenizer implementations
mod tokenizer_hf;       // HuggingFace tokenizer (legacy - for non-GGUF models only)
mod tokenizer_gguf_core;     // GGUF tokenizer core definitions
mod tokenizer_gguf_encoding; // GGUF tokenizer encoding logic
mod tokenizer_gguf_decoding; // GGUF tokenizer decoding logic
mod tokenizer_gguf_loader;   // GGUF tokenizer loader functions
mod tokenizer_data;     // Generated tokenizer data (legacy - for non-GGUF models only)
mod embedded_model;     // Generated model data
mod tokenizer_debug;    // Debug tools for tokenizer analysis
mod utils;

// Re-export everything
pub use candle_model_core::*;
pub use tokenizer_hf::*;
pub use tokenizer_gguf_core::*;
pub use candle_transformer_layer::*;
pub use embedded_model::EMBEDDED_MODEL;
pub use tokenizer_debug::*;
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
    gguf_tokenizer: Option<GgufTokenizer>,
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
            using_gguf_tokenizer: false, // Default to HuggingFace tokenizer for reliable tokenization
        }
    }
    
    /// Creates a new Qwen3Client using HuggingFace tokenizer explicitly
    /// This option is available when you want to use a specific HuggingFace tokenizer
    /// instead of the one embedded in the GGUF model
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
        let init_start = std::time::Instant::now();
        
        // Get detailed model information for metrics
        // Format: base model name, size/quantization format, total size in MB
        let model_size_mb = crate::EMBEDDED_MODEL.len() / (1024 * 1024);
        
        // Detect the current model variant from embedded_model.rs
        let (model_base, quant_type) = self.detect_model_variant();
        let model_format = "gguf";
        let model_name = format!("{}-{}.{}", model_base, quant_type, model_format);
        
        // Get device information
        let device_name = if self.config.use_gpu {
            if cfg!(target_os = "macos") { "metal" } else { "cuda" }
        } else {
            "cpu"
        };
        
        // Get context window size (typical for Qwen3-0.6B)
        let context_window = 2048;
        
        // Log detailed model information and device at startup
        eprintln!("[METRICS] model={} size={}MB ctx={} device={}", 
                 model_name, model_size_mb, context_window, device_name);
        
        // Initialize the Candle model - always use embedded model
        // which is included directly via include_bytes!
        match CandleQwen3Model::new_from_embedded(&self.config) {
            Ok(model) => {
                self.model = Some(model);
            },
            Err(err) => {
                return Err(anyhow::anyhow!("Failed to load embedded Qwen3 model: {}", err));
            }
        }
        
        // For testing purposes, we can try to load the GGUF tokenizer if explicitly requested
        if self.using_gguf_tokenizer {
            match GgufTokenizer::new(crate::EMBEDDED_MODEL) {
                Ok(tokenizer) => {
                    // Do a simple verification to make sure basic tokens are present
                    let basic_token_test = [
                        (" ", 220),     // Space token ID (GPT-2 style)
                        ("!", 0),       // Exclamation mark - should be present
                        ("<|im_start|>", 151644), // ChatML start token
                        ("<|im_end|>", 151645),   // ChatML end token
                    ];
                    
                    // Quick verification for critical tokens to validate the tokenizer
                    let mut has_missing_tokens = false;
                    for (token_str, expected_id) in &basic_token_test {
                        match tokenizer.encode(token_str) {
                            Ok(ids) if ids.contains(expected_id) || expected_id == &0 => {
                                // Token verified successfully or we're checking for UNK (0)
                                if token_str == &"!" && ids.contains(&0) {
                                    // Found an issue - exclamation mark is UNK
                                    has_missing_tokens = true;
                                    eprintln!("⚠️ WARNING: '!' character produces UNK tokens with GGUF tokenizer");
                                }
                            },
                            Ok(ids) => {
                                eprintln!("⚠️ WARNING: Token '{}' found but with ID {:?} (expected {})",
                                          token_str, ids, expected_id);
                            },
                            Err(_) => {
                                has_missing_tokens = true;
                                eprintln!("⚠️ WARNING: Token '{}' failed to encode with GGUF tokenizer", token_str);
                            }
                        }
                    }
                    
                    // If critical tokens are missing, show a more detailed warning
                    if has_missing_tokens {
                        eprintln!("⚠️ WARNING: GGUF tokenizer verification failed. Falling back to HuggingFace tokenizer.");
                        self.using_gguf_tokenizer = false;
                    } else {
                        self.gguf_tokenizer = Some(tokenizer);
                    }
                },
                Err(e) => {
                    // Fall back to HuggingFace tokenizer
                    eprintln!("⚠️ WARNING: Failed to load GGUF tokenizer: {}. Falling back to HuggingFace tokenizer.", e);
                    self.using_gguf_tokenizer = false;
                }
            }
        }
        
        // Fall back to HuggingFace tokenizer if GGUF tokenizer initialization failed
        // or if HuggingFace tokenizer was explicitly requested
        if !self.using_gguf_tokenizer {
            // Attempt to load from the models directory first
            let tokenizer_paths = [
                "models/tokenizer.json",
                "./models/tokenizer.json",
                "./crates/arkavo-llm/models/tokenizer.json",
                "../crates/arkavo-llm/models/tokenizer.json",
            ];
            
            let mut loaded_from_file = false;
            
            // Try loading from file paths first
            for path in &tokenizer_paths {
                if std::path::Path::new(path).exists() {
                    match tokenizer_hf::HfTokenizer::new(path) {
                        Ok(tokenizer) => {
                            eprintln!("✓ HuggingFace tokenizer loaded from {}", path);
                            self.hf_tokenizer = Some(tokenizer);
                            loaded_from_file = true;
                            break;
                        },
                        Err(e) => {
                            eprintln!("⚠️ Failed to load HF tokenizer from {}: {}", path, e);
                        }
                    }
                }
            }
            
            // Fall back to embedded JSON if file loading failed
            if !loaded_from_file {
                eprintln!("Attempting to load HF tokenizer from embedded bytes");
                match tokenizer_hf::HfTokenizer::from_bytes(utils::EMBEDDED_TOKENIZER_JSON) {
                    Ok(tokenizer) => {
                        eprintln!("✓ HuggingFace tokenizer loaded from embedded bytes");
                        self.hf_tokenizer = Some(tokenizer);
                    },
                    Err(err) => {
                        return Err(anyhow::anyhow!("Failed to load any tokenizer: {}", err));
                    }
                }
            }
            
            // Ensure we have a tokenizer
            if self.hf_tokenizer.is_none() {
                return Err(anyhow::anyhow!("Failed to load any tokenizer"));
            }
        }
        
        // Log initialization time with standardized format
        eprintln!("[METRICS] load={:.2}s", init_start.elapsed().as_secs_f64());

        Ok(())
    }

    /// Generates text completion for the given prompt
    pub async fn generate(&self, prompt: &str) -> Result<String> {
        // Format the prompt using Qwen3's expected chat template
        let formatted_prompt = self.format_qwen3_prompt(prompt);
        
        // Start timing the overall inference process
        let inference_start = std::time::Instant::now();
        
        // Use the appropriate tokenizer based on what was initialized
        let input_tokens = if self.using_gguf_tokenizer {
            // Use GGUF tokenizer
            let tokenizer = self.gguf_tokenizer
                .as_ref()
                .ok_or_else(|| LlmError::InferenceError("GGUF tokenizer not initialized".to_string()))?;
                
            let tokens = tokenizer.encode(&formatted_prompt)?;
            
            // Validate that our tokenization doesn't have UNK tokens (id 0)
            let unk_count = tokens.iter().filter(|&&id| id == 0).count();
            if unk_count > 0 && (unk_count as f32 / tokens.len() as f32) > 0.05 {
                // Only warn for significant unknown token rates (>5%)
                eprintln!("⚠️ Warning: Input contains {}% unknown tokens", 
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
        
        // Generate response using Candle model
        let model = self.model
            .as_ref()
            .ok_or_else(|| LlmError::InferenceError("Model not initialized".to_string()))?;
        
        // Check that we have input tokens
        if input_tokens.is_empty() {
            return Err(anyhow::anyhow!("No input tokens generated - check tokenizer configuration"));
        }
            
        // Log forward pass timing
        let generation_start = std::time::Instant::now();
        let all_tokens = model.generate(&input_tokens, self.config.max_tokens)?;
        let generation_duration = generation_start.elapsed();
        
        // Extract only the newly generated tokens (excluding the input prompt)
        let output_tokens = if all_tokens.len() > input_tokens.len() {
            all_tokens[input_tokens.len()..].to_vec()
        } else {
            vec![]
        };
        
        // Calculate tokens per second
        let tokens_per_second = if generation_duration.as_secs_f64() > 0.0 {
            output_tokens.len() as f64 / generation_duration.as_secs_f64()
        } else {
            0.0 // Avoid division by zero
        };
        
        // If no tokens were generated, return an error
        if output_tokens.is_empty() {
            return Err(anyhow::anyhow!("No tokens generated by the model"));
        }
        
        // Log critical issues only
        let unk_count = output_tokens.iter().filter(|&&id| id == 0).count();
        if unk_count > 0 && (unk_count as f32 / output_tokens.len() as f32) > 0.05 {
            eprintln!("⚠️ Warning: Output contains {}% unknown tokens", 
                     (unk_count as f32 / output_tokens.len() as f32) * 100.0);
        }
        
        // Decode response using the same tokenizer used for encoding
        let raw_response = if self.using_gguf_tokenizer {
            // Use GGUF tokenizer
            let tokenizer = self.gguf_tokenizer
                .as_ref()
                .ok_or_else(|| LlmError::InferenceError("GGUF tokenizer not initialized".to_string()))?;
            
            tokenizer.decode(&output_tokens)?
        } else {
            // Fall back to HuggingFace tokenizer
            let tokenizer = self.hf_tokenizer
                .as_ref()
                .ok_or_else(|| LlmError::InferenceError("HuggingFace tokenizer not initialized".to_string()))?;
            
            tokenizer.decode(&output_tokens)?
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
        
        // Log total inference metrics
        let total_duration = inference_start.elapsed();
        
        // Calculate input prompt length in tokens for context metrics
        let prompt_tokens = input_tokens.len();
        let total_tokens = prompt_tokens + output_tokens.len();
        
        // Print standardized metrics log with context information
        eprintln!(
            "[METRICS] infer={:.2}s total={:.2}s prompt_tokens={} gen_tokens={} total_tokens={} tps={:.2}",
            generation_duration.as_secs_f64(),
            total_duration.as_secs_f64(),
            prompt_tokens,
            output_tokens.len(),
            total_tokens,
            tokens_per_second
        );

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
    /// Note: This is now unused as we use the formatted prompt directly from the chat module
    fn format_qwen3_prompt(&self, user_prompt: &str) -> String {
        // For simple prompts, we'll use Qwen3's ChatML format
        // This follows the structure:
        // <|im_start|>system
        // [system prompt]<|im_end|>
        // <|im_start|>user
        // [user message]<|im_end|>
        // <|im_start|>assistant
        
        user_prompt.to_string()
    }
    
    /// Detects the model variant and quantization type from embedded_model.rs
    fn detect_model_variant(&self) -> (String, String) {
        // Default values if detection fails
        let model_base = "qwen3-0.6b".to_string();
        let mut quant_type = "Q4_K_M".to_string();
        
        // Try to detect from the embedded model.rs file path
        // This requires looking at the source code or using a regex
        // As a fallback, we can use the file size to roughly identify the model
        
        // Model size heuristics for different quantization levels:
        // - F16: ~1.2GB
        // - Q8_0: ~600MB
        // - Q5_K_M: ~500MB
        // - Q4_K_M: ~460MB (current model)
        // - Q2_K: ~350MB
        
        let model_size_mb = crate::EMBEDDED_MODEL.len() / (1024 * 1024);
        
        // Use size-based heuristics to detect model variant
        if model_size_mb > 1000 {
            quant_type = "F16".to_string();
        } else if model_size_mb > 550 {
            quant_type = "Q8_0".to_string();
        } else if model_size_mb > 480 {
            quant_type = "Q5_K_M".to_string();
        } else if model_size_mb > 400 {
            quant_type = "Q4_K_M".to_string();
        } else if model_size_mb > 300 {
            quant_type = "Q2_K".to_string();
        }
        
        // For more accurate detection, we could inspect the GGUF header/metadata
        // But that would require parsing the GGUF file
        
        (model_base, quant_type)
    }
}