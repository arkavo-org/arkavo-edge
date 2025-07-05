use crate::{Error, Result};
use candle_core::quantized::gguf_file;
use candle_core::{Device, Tensor};
use std::io::Seek;
use std::path::Path;
use std::sync::Arc;
use tokenizers::Tokenizer;

// Import the specific quantized models
use candle_transformers::models::llama;
use candle_transformers::models::quantized_gemma3;
use candle_transformers::models::quantized_llama;
use candle_transformers::models::quantized_phi;

pub enum Model {
    QuantizedGemma3(quantized_gemma3::ModelWeights),
    QuantizedLlama(quantized_llama::ModelWeights),
    QuantizedPhi(quantized_phi::ModelWeights),
}

pub struct ModelLoader {
    device: Device,
    model_name: String,
    model_path: Option<String>,
    model: Option<Model>,
    config: Option<llama::Config>,
    tokenizer_path: Option<String>,
    tokenizer: Option<Arc<Tokenizer>>,
    eos_token_id: Option<u32>,
}

impl ModelLoader {
    pub fn new(model_name: &str, model_path: Option<&str>) -> Result<Self> {
        // Select device based on platform
        let device = Self::select_device()?;

        Ok(Self {
            device,
            model_name: model_name.to_string(),
            model_path: model_path.map(String::from),
            model: None,
            config: None,
            tokenizer_path: None,
            tokenizer: None,
            eos_token_id: None,
        })
    }

    fn select_device() -> Result<Device> {
        cfg_if::cfg_if! {
            if #[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "metal"))] {
                // Try Metal first, fall back to CPU
                match Device::new_metal(0) {
                    Ok(device) => {
                        tracing::info!("Using Metal device for acceleration");
                        Ok(device)
                    }
                    Err(_) => {
                        tracing::warn!("Metal device not available, falling back to CPU");
                        Ok(Device::Cpu)
                    }
                }
            } else {
                // Default to CPU for other platforms or when Metal feature is disabled
                tracing::info!("Using CPU device");
                Ok(Device::Cpu)
            }
        }
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    pub fn load_model(&mut self) -> Result<()> {
        let path = self
            .model_path
            .clone()
            .ok_or_else(|| Error::Config("Model path not specified".to_string()))?;

        let model_path = Path::new(&path);
        if !model_path.exists() {
            return Err(Error::Config(format!("Model file not found: {path}")));
        }

        eprintln!(
            "Loading model '{}' on device {:?}",
            self.model_name, self.device
        );

        // Check if it's a GGUF file
        if path.ends_with(".gguf") || path.ends_with(".GGUF") {
            // Try to load as quantized model first
            match self.load_quantized_model(model_path) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    eprintln!("Failed to load as quantized model: {}", e);
                    return Err(e);
                }
            }
        }

        // Fallback to standard model loading
        Err(Error::Model(
            "Non-quantized model loading not yet implemented".to_string(),
        ))
    }

    fn load_quantized_model(&mut self, path: &Path) -> Result<()> {
        use candle_core::quantized::gguf_file;

        // Load GGUF file - open once and clone for reuse
        let mut file = std::fs::File::open(path)
            .map_err(|e| Error::Model(format!("Failed to open model file: {e}")))?;

        let content = gguf_file::Content::read(&mut file)
            .map_err(|e| Error::Model(format!("Failed to read GGUF file: {e}")))?;

        // Extract architecture from metadata
        let arch = content
            .metadata
            .get("general.architecture")
            .and_then(|v| {
                if let gguf_file::Value::String(s) = v {
                    Some(s.to_string())
                } else {
                    None
                }
            })
            .or_else(|| {
                // Fallback: check for phi-specific metadata
                if content.metadata.contains_key("phi.context_length")
                    || content.metadata.contains_key("phi2.context_length")
                    || self.model_name.to_lowercase().contains("phi")
                {
                    Some("phi".to_string())
                } else {
                    None
                }
            })
            .ok_or_else(|| Error::Model("Missing architecture in GGUF metadata".to_string()))?;

        eprintln!("Detected GGUF architecture: {}", arch);

        // Check if quantization is compatible with Metal
        let mut device = self.device.clone();
        if matches!(device, Device::Metal(_)) {
            // GGUF models with quantization don't work well on Metal yet
            tracing::warn!(
                "GGUF quantized models are not yet fully optimized for Metal. \
                 Using CPU for better compatibility."
            );
            device = Device::Cpu;
        }

        // Extract config from model metadata first
        let eos_token_id = self.extract_config_from_gguf(&content);
        if let Some(eos_id) = eos_token_id {
            self.eos_token_id = Some(eos_id);
            tracing::debug!("Found EOS token ID in GGUF metadata: {}", eos_id);
        }

        // The unified, architecture-aware loading logic
        let model = match arch.as_str() {
            "gemma3" => {
                eprintln!("Loading Gemma 3 GGUF model...");
                // Seek back to the beginning after reading metadata
                file.seek(std::io::SeekFrom::Start(0))?;
                let model = quantized_gemma3::ModelWeights::from_gguf(content, &mut file, &device)
                    .map_err(|e| Error::Model(format!("Failed to load Gemma3 model: {e}")))?;
                Model::QuantizedGemma3(model)
            }
            "llama" => {
                eprintln!("Loading Llama GGUF model...");
                // Seek back to the beginning after reading metadata
                file.seek(std::io::SeekFrom::Start(0))?;
                let model =
                    quantized_llama::ModelWeights::from_gguf(content, &mut file, &device)
                        .map_err(|e| Error::Model(format!("Failed to load Llama model: {e}")))?;
                Model::QuantizedLlama(model)
            }
            "phi" | "phi2" => {
                eprintln!("Loading Phi GGUF model...");
                // Seek back to the beginning after reading metadata
                file.seek(std::io::SeekFrom::Start(0))?;
                let model = quantized_phi::ModelWeights::from_gguf(content, &mut file, &device)
                    .map_err(|e| Error::Model(format!("Failed to load Phi model: {e}")))?;
                Model::QuantizedPhi(model)
            }
            _ => return Err(Error::Model(format!("Unsupported architecture: {}", arch))),
        };

        self.model = Some(model);
        self.device = device; // Update device if it changed

        eprintln!("Successfully loaded {} model", arch);
        Ok(())
    }

    fn extract_config_from_gguf(
        &mut self,
        content: &candle_core::quantized::gguf_file::Content,
    ) -> Option<u32> {
        use candle_core::quantized::gguf_file::Value;

        let metadata = &content.metadata;

        // Extract tokenizer data if available
        self.extract_tokenizer_from_gguf(content);

        // Extract model parameters from metadata
        let hidden_size = metadata
            .get("llama.embedding_length")
            .and_then(|v| {
                if let Value::U32(n) = v {
                    Some(*n as usize)
                } else {
                    None
                }
            })
            .unwrap_or(2048);

        let num_hidden_layers = metadata
            .get("llama.block_count")
            .and_then(|v| {
                if let Value::U32(n) = v {
                    Some(*n as usize)
                } else {
                    None
                }
            })
            .unwrap_or(18);

        let num_attention_heads = metadata
            .get("llama.attention.head_count")
            .and_then(|v| {
                if let Value::U32(n) = v {
                    Some(*n as usize)
                } else {
                    None
                }
            })
            .unwrap_or(16);

        let vocab_size = metadata
            .get("tokenizer.ggml.tokens")
            .and_then(|v| {
                if let Value::Array(arr) = v {
                    Some(arr.len())
                } else {
                    None
                }
            })
            .unwrap_or(32000);

        let max_position_embeddings = metadata
            .get("llama.context_length")
            .and_then(|v| {
                if let Value::U32(n) = v {
                    Some(*n as usize)
                } else {
                    None
                }
            })
            .unwrap_or(8192);

        let rope_theta = metadata
            .get("llama.rope.theta")
            .and_then(|v| {
                if let Value::F32(n) = v {
                    Some(*n as f64)
                } else {
                    None
                }
            })
            .unwrap_or(10_000.0);

        let config = llama::Config {
            hidden_size,
            intermediate_size: hidden_size * 4, // Standard ratio
            vocab_size,
            num_hidden_layers,
            num_attention_heads,
            num_key_value_heads: 1, // GQA default
            max_position_embeddings,
            rms_norm_eps: 1e-6,
            rope_theta: rope_theta as f32,
            tie_word_embeddings: false,
            use_flash_attn: false,
            bos_token_id: None,
            eos_token_id: None,
            rope_scaling: None,
        };

        self.config = Some(config);

        // Try to find EOS token ID from metadata
        metadata
            .get("tokenizer.ggml.eos_token_id")
            .and_then(|v| {
                if let Value::U32(id) = v {
                    Some(*id)
                } else {
                    None
                }
            })
            .or_else(|| {
                // Fallback: look for EOS in token types
                if let Some(Value::Array(token_types)) = metadata.get("tokenizer.ggml.token_type") {
                    // EOS token type is typically 3
                    token_types
                        .iter()
                        .position(|v| {
                            if let Value::U32(token_type) = v {
                                *token_type == 3
                            } else {
                                false
                            }
                        })
                        .map(|pos| pos as u32)
                } else {
                    None
                }
            })
    }

    fn extract_tokenizer_from_gguf(
        &mut self,
        content: &candle_core::quantized::gguf_file::Content,
    ) {
        let metadata = &content.metadata;

        // Try multiple approaches to get a tokenizer

        // 1. Try embedded tokenizer model (check if it's actually binary data)
        if let Some(value) = metadata.get("tokenizer.ggml.model") {
            // Check if it's actually binary data or just a string marker
            match value {
                gguf_file::Value::String(s) if s == "llama" => {
                    // This is just a marker, not actual tokenizer data
                    tracing::debug!("Found 'llama' marker instead of embedded tokenizer");
                }
                _ => {
                    // Try to load as embedded tokenizer
                    if self.try_embedded_tokenizer(value) {
                        return;
                    }
                }
            }
        }

        // 2. For Gemma models, try to construct a simple tokenizer from metadata
        // This is a temporary solution until we can bundle the actual tokenizer
        if self.try_construct_tokenizer_from_metadata(content) {
            return;
        }

        // 3. Fallback: look for tokenizer.model alongside the .gguf
        if let Some(model_path) = &self.model_path {
            let tokenizer_path = Path::new(model_path).with_file_name("tokenizer.model");
            if tokenizer_path.exists() {
                match Tokenizer::from_file(&tokenizer_path) {
                    Ok(tokenizer) => {
                        self.tokenizer = Some(Arc::new(tokenizer));
                        self.tokenizer_path = Some(tokenizer_path.to_string_lossy().into_owned());
                        tracing::info!("Loaded tokenizer from file: tokenizer.model");
                        return;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load tokenizer from file: {}", e);
                    }
                }
            }

            // 4. Try tokenizer.json for Gemma models
            let tokenizer_json_path = Path::new(model_path).with_file_name("tokenizer.json");
            if tokenizer_json_path.exists() {
                match Tokenizer::from_file(&tokenizer_json_path) {
                    Ok(tokenizer) => {
                        self.tokenizer = Some(Arc::new(tokenizer));
                        self.tokenizer_path =
                            Some(tokenizer_json_path.to_string_lossy().into_owned());
                        tracing::info!("Loaded tokenizer from file: tokenizer.json");
                        return;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load tokenizer from JSON file: {}", e);
                    }
                }
            }
        }

        tracing::error!(
            "Could not create tokenizer for model {}. GGUF file may not contain embedded tokenizer. \
            Please ensure tokenizer.model exists alongside the GGUF file.",
            self.model_name
        );
    }

    fn try_construct_tokenizer_from_metadata(
        &mut self,
        content: &candle_core::quantized::gguf_file::Content,
    ) -> bool {
        // For now, we'll create a simple tokenizer that can at least encode/decode basic text
        // This is a temporary solution until we can bundle the actual Gemma tokenizer

        // Check if we have the token list
        if let Some(gguf_file::Value::Array(tokens)) = content.metadata.get("tokenizer.ggml.tokens")
        {
            // For now, just log that we found tokens but can't construct a full tokenizer
            tracing::warn!(
                "Found {} tokens in GGUF metadata, but cannot construct a full tokenizer without the SentencePiece model. \
                Please download tokenizer.model separately.",
                tokens.len()
            );

            // TODO: When we have the bundled tokenizer, we'll load it here
            // let tokenizer_bytes = include_bytes!("../assets/gemma_tokenizer.model");
            // let tokenizer = Tokenizer::from_bytes(tokenizer_bytes)?;
        }

        false
    }

    fn try_embedded_tokenizer(&mut self, value: &candle_core::quantized::gguf_file::Value) -> bool {
        use candle_core::quantized::gguf_file::Value;

        let bytes = match value {
            // Handle Array of U8 values (less common)
            Value::Array(values) => {
                let mut bytes = Vec::new();
                for val in values {
                    if let Value::U8(byte) = val {
                        bytes.push(*byte);
                    }
                }
                if bytes.is_empty() {
                    tracing::debug!("Array format tokenizer is empty");
                    return false;
                }
                tracing::debug!("Found tokenizer as Array of U8, {} bytes", bytes.len());
                bytes
            }
            _ => {
                tracing::debug!("Embedded tokenizer not in expected Binary or Array format");
                return false;
            }
        };

        // Write to temporary file and load tokenizer
        let temp_dir = std::env::temp_dir();
        let tokenizer_path = temp_dir.join(format!(
            "arkavo_tokenizer_{}_{}.spm",
            self.model_name,
            std::process::id()
        ));

        if let Ok(()) = std::fs::write(&tokenizer_path, &bytes) {
            match Tokenizer::from_file(&tokenizer_path) {
                Ok(tokenizer) => {
                    self.tokenizer = Some(Arc::new(tokenizer));
                    self.tokenizer_path = Some(tokenizer_path.to_string_lossy().into_owned());
                    tracing::info!("Successfully loaded embedded tokenizer from GGUF");

                    // Clean up temp file
                    let _ = std::fs::remove_file(&tokenizer_path);
                    return true;
                }
                Err(e) => {
                    tracing::error!("Failed to load tokenizer from temp file: {}", e);
                    let _ = std::fs::remove_file(&tokenizer_path);
                }
            }
        }
        false
    }

    pub fn create_tensor(&self, data: &[f32], shape: &[usize]) -> Result<Tensor> {
        Tensor::from_slice(data, shape, &self.device)
            .map_err(|e| Error::Model(format!("Failed to create tensor: {e}")))
    }

    pub fn get_model(&self) -> Option<&Model> {
        self.model.as_ref()
    }

    pub fn get_model_mut(&mut self) -> Option<&mut Model> {
        self.model.as_mut()
    }

    pub fn get_config(&self) -> Option<&llama::Config> {
        self.config.as_ref()
    }

    pub fn get_context_length(&self) -> u32 {
        self.config
            .as_ref()
            .map(|c| c.max_position_embeddings as u32)
            .unwrap_or(2048)
    }

    pub fn is_loaded(&self) -> bool {
        self.model.is_some()
    }

    pub fn set_tokenizer_path(&mut self, path: String) {
        self.tokenizer_path = Some(path);
    }

    pub fn get_tokenizer_path(&self) -> Option<&str> {
        self.tokenizer_path.as_deref()
    }

    pub fn tokenizer(&self) -> Result<Arc<Tokenizer>> {
        self.tokenizer
            .clone()
            .ok_or_else(|| Error::Model("Tokenizer not loaded".into()))
    }

    pub fn has_tokenizer(&self) -> bool {
        self.tokenizer.is_some()
    }

    pub fn eos_token_id(&self) -> Option<u32> {
        self.eos_token_id
    }

    pub fn context_length(&self) -> usize {
        // Get context length from config if available
        self.config
            .as_ref()
            .map(|c| c.max_position_embeddings as usize)
            .unwrap_or(2048) // Default to 2048 if not found
    }
}
