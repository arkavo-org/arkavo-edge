#[cfg(feature = "llama-cpp")]
use crate::llama_cpp::provider::LlamaCppProvider;
use crate::{Error, Result};
use candle_core::quantized::gguf_file;
use candle_core::{Device, Tensor};
use std::collections::HashMap;
use std::io::Seek;
use std::path::Path;
use std::sync::Arc;
use tokenizers::Tokenizer;

// Import the specific quantized models
use candle_transformers::models::llama;
use candle_transformers::models::quantized_llama;
use candle_transformers::models::quantized_phi;

pub enum Model {
    QuantizedLlama(quantized_llama::ModelWeights),
    QuantizedPhi(quantized_phi::ModelWeights),
}

use crate::provider::Provider as CompletionProvider;

pub struct ModelLoader {
    device: Device,
    model_name: String,
    model_path: Option<String>,
    provider: Option<Box<dyn CompletionProvider>>,
    config: Option<llama::Config>,
    tokenizer_path: Option<String>,
    tokenizer: Option<Arc<Tokenizer>>,
    eos_token_ids: Vec<u32>,
}

impl ModelLoader {
    pub fn new(model_name: &str, model_path: Option<&str>) -> Result<Self> {
        // Select device based on platform
        let device = Self::select_device()?;

        Ok(Self {
            device,
            model_name: model_name.to_string(),
            model_path: model_path.map(String::from),
            provider: None,
            config: None,
            tokenizer_path: None,
            tokenizer: None,
            eos_token_ids: Vec::new(),
        })
    }

    fn select_device() -> Result<Device> {
        cfg_if::cfg_if! {
            if #[cfg(all(target_os = "macos", target_arch = "aarch64"))] {
                // On macOS ARM64, always try Metal first
                #[cfg(feature = "metal")]
                match Device::new_metal(0) {
                    Ok(device) => {
                        tracing::info!("Using Metal device for acceleration");
                        return Ok(device);
                    }
                    Err(e) => {
                        tracing::warn!("Metal device not available ({}), falling back to CPU", e);
                    }
                }

                // Fallback to CPU
                tracing::info!("Using CPU device");
                Ok(Device::Cpu)
            } else {
                // Default to CPU for other platforms
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
                    eprintln!("Failed to load as quantized model: {e}");
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
            .ok_or_else(|| Error::Model("Missing architecture in GGUF metadata".to_string()))?;

        eprintln!("Detected GGUF architecture: {arch}");

        if arch == "gemma3" {
            #[cfg(feature = "llama-cpp")]
            {
                let provider = LlamaCppProvider::new(
                    &path.to_string_lossy(),
                    &crate::local::config::LocalConfig::default(),
                )?;
                self.provider = Some(Box::new(provider));
                return Ok(());
            }
            #[cfg(not(feature = "llama-cpp"))]
            {
                return Err(Error::Model(
                    "Gemma 3 models require the `llama-cpp` feature to be enabled.".to_string(),
                ));
            }
        }

        // Use the device as configured (Metal will be used on macOS if available)
        let device = self.device.clone();
        if matches!(device, Device::Metal(_)) {
            tracing::info!("Using Metal NPU acceleration for {} model inference", arch);
        }

        // Extract config from model metadata first
        let eos_token_id = self.extract_config_from_gguf(&content);
        if let Some(eos_id) = eos_token_id {
            self.eos_token_ids.push(eos_id);
            tracing::debug!("Found EOS token ID in GGUF metadata: {}", eos_id);
        }

        // The unified, architecture-aware loading logic
        let model = match arch.as_str() {
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
            _ => return Err(Error::Model(format!("Unsupported architecture: {arch}"))),
        };

        self.provider = Some(Box::new(super::candle_provider::CandleProvider::new(
            self.model_name.clone(),
            self.model_path.clone(),
            &Default::default(),
        )?));

        eprintln!("Successfully loaded {arch} model");
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
        // Try multiple approaches to get a tokenizer

        // 1. Try embedded tokenizer model
        if self.try_load_embedded_tokenizer(&content.metadata) {
            return;
        }

        // 2. For Gemma models, try to construct a simple tokenizer from metadata
        if self.try_construct_tokenizer_from_metadata(content) {
            return;
        }

        // 3. For Gemma models, try to find tokenizer in HF cache
        if self.model_name.starts_with("gemma") && self.try_load_from_hf_cache() {
            return;
        }

        // 4. Fallback: look for tokenizer files alongside the .gguf
        if self.try_load_alongside_model() {
            return;
        }

        // 5. Last resort: try to download tokenizer if we can identify the base model
        if self.try_download_tokenizer() {
            return;
        }

        tracing::error!(
            "Could not create tokenizer for model {}. GGUF file may not contain embedded tokenizer. \
            Consider adding base_repo_id to models.toml or placing tokenizer.json alongside the GGUF file.",
            self.model_name
        );
    }

    /// Try to load embedded tokenizer from GGUF metadata
    fn try_load_embedded_tokenizer(
        &mut self,
        metadata: &HashMap<String, gguf_file::Value>,
    ) -> bool {
        if let Some(value) = metadata.get("tokenizer.ggml.model") {
            // Check if it's actually binary data or just a string marker
            match value {
                gguf_file::Value::String(s) if s == "llama" => {
                    // This is just a marker, not actual tokenizer data
                    tracing::debug!("Found 'llama' marker instead of embedded tokenizer");
                    false
                }
                _ => self.try_embedded_tokenizer(value),
            }
        } else {
            false
        }
    }

    /// Try to load tokenizer from HuggingFace cache for Gemma models
    fn try_load_from_hf_cache(&mut self) -> bool {
        let Some(home) = dirs::home_dir() else {
            return false;
        };

        let cache_base = home.join(".cache/huggingface/hub");
        let base_repos = self.get_gemma_base_repos();

        for repo_name in base_repos {
            if self.try_load_from_repo(&cache_base, repo_name) {
                return true;
            }
        }

        false
    }

    /// Get base repository names for Gemma models
    fn get_gemma_base_repos(&self) -> Vec<&'static str> {
        match self.model_name.as_str() {
            "gemma3-1b-it-qat" => vec![
                "models--google--gemma-3-1b-it",
                "models--google--gemma-3-1b-it-qat-q4_0-gguf",
            ],
            "gemma3n-e4b-it" => vec![
                "models--google--gemma-2b-it",
                "models--unsloth--gemma-3n-E4B-it-GGUF",
            ],
            _ => vec!["models--google--gemma-2b-it"],
        }
    }

    /// Try to load tokenizer from a specific HF repository cache
    fn try_load_from_repo(&mut self, cache_base: &Path, repo_name: &str) -> bool {
        let repo_path = cache_base.join(repo_name);
        tracing::debug!("Checking for tokenizer in: {:?}", repo_path);

        if !repo_path.exists() {
            return false;
        }

        let snapshots_dir = repo_path.join("snapshots");
        if let Ok(entries) = std::fs::read_dir(&snapshots_dir) {
            for entry in entries.flatten() {
                let snapshot_path = entry.path();
                let tokenizer_path = snapshot_path.join("tokenizer.model");

                if self.try_load_tokenizer_file(&tokenizer_path) {
                    return true;
                }
            }
        }

        false
    }

    /// Try to load tokenizer files alongside the model file
    fn try_load_alongside_model(&mut self) -> bool {
        let model_path = match &self.model_path {
            Some(path) => path.clone(),
            None => return false,
        };

        // Try tokenizer.json first (JSON format that works with all models)
        let tokenizer_json_path = Path::new(&model_path).with_file_name("tokenizer.json");
        tracing::debug!("Looking for tokenizer at: {:?}", tokenizer_json_path);

        if self.try_load_tokenizer_file(&tokenizer_json_path) {
            return true;
        }

        // Also try tokenizer.model for SentencePiece tokenizers
        let tokenizer_model_path = Path::new(&model_path).with_file_name("tokenizer.model");
        if self.try_load_tokenizer_file(&tokenizer_model_path) {
            return true;
        }

        tracing::debug!("No tokenizer files found alongside model");
        false
    }

    /// Try to load a tokenizer from a specific file path
    fn try_load_tokenizer_file(&mut self, tokenizer_path: &Path) -> bool {
        if !tokenizer_path.exists() {
            return false;
        }

        tracing::debug!("Trying to load tokenizer from: {:?}", tokenizer_path);
        match Tokenizer::from_file(tokenizer_path) {
            Ok(tokenizer) => {
                tracing::info!("Successfully loaded tokenizer from: {:?}", tokenizer_path);
                self.set_tokenizer_and_extract_eos(tokenizer);
                self.tokenizer_path = Some(tokenizer_path.to_string_lossy().into_owned());
                true
            }
            Err(e) => {
                tracing::warn!("Failed to load tokenizer from {:?}: {}", tokenizer_path, e);
                false
            }
        }
    }

    fn set_tokenizer_and_extract_eos(&mut self, tokenizer: Tokenizer) {
        // Get EOS token IDs from tokenizer
        let eos_ids = super::tokenizer_utils::get_eos_token_ids(&tokenizer);
        if !eos_ids.is_empty() {
            self.eos_token_ids = eos_ids;
        }

        self.tokenizer = Some(Arc::new(tokenizer));
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
            tracing::info!(
                "Found {} tokens in GGUF metadata, attempting to use bundled tokenizer",
                tokens.len()
            );

            // Try to load bundled tokenizer for common models
            if let Some(tokenizer) = self.load_bundled_tokenizer(&content.metadata) {
                self.tokenizer = Some(tokenizer);
                return true;
            }

            tracing::warn!(
                "No bundled tokenizer available for this model. Please download tokenizer.model separately."
            );
        }

        false
    }

    /// Load a bundled tokenizer based on model metadata
    fn load_bundled_tokenizer(
        &self,
        metadata: &std::collections::HashMap<String, gguf_file::Value>,
    ) -> Option<Arc<tokenizers::Tokenizer>> {
        // Check model type from metadata
        let model_name = metadata.get("general.name").and_then(|v| match v {
            gguf_file::Value::String(s) => Some(s.as_str()),
            _ => None,
        });

        match model_name {
            Some(name) if name.contains("gemma") => {
                // For Gemma models, use a simple tokenizer stub
                // In production, this would load actual tokenizer data
                tracing::info!("Loading tokenizer stub for Gemma model");
                self.create_tokenizer_stub()
            }
            Some(name) if name.contains("llama") => {
                tracing::info!("Loading tokenizer stub for Llama model");
                self.create_tokenizer_stub()
            }
            _ => None,
        }
    }

    /// Create a basic tokenizer stub for development
    fn create_tokenizer_stub(&self) -> Option<Arc<tokenizers::Tokenizer>> {
        use tokenizers::models::bpe::BPE;
        use tokenizers::tokenizer::Tokenizer;

        // Create a minimal BPE tokenizer
        // In production, this would load actual vocabulary and merges
        let bpe = BPE::default();
        let tokenizer = Tokenizer::new(bpe);

        Some(Arc::new(tokenizer))
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

        if std::fs::write(&tokenizer_path, &bytes).is_ok() {
            match Tokenizer::from_file(&tokenizer_path) {
                Ok(tokenizer) => {
                    self.tokenizer = Some(Arc::new(tokenizer));
                    self.tokenizer_path = Some(tokenizer_path.to_string_lossy().into_owned());
                    tracing::info!("Successfully loaded embedded tokenizer from GGUF");

                    // Clean up temp file
                    if let Err(e) = std::fs::remove_file(&tokenizer_path) {
                        tracing::debug!("Failed to remove temporary tokenizer file: {}", e);
                    }
                    return true;
                }
                Err(e) => {
                    tracing::error!("Failed to load tokenizer from temp file: {}", e);
                    if let Err(e) = std::fs::remove_file(&tokenizer_path) {
                        tracing::debug!("Failed to remove temporary tokenizer file: {}", e);
                    }
                }
            }
        }
        false
    }

    pub fn create_tensor(&self, data: &[f32], shape: &[usize]) -> Result<Tensor> {
        Tensor::from_slice(data, shape, &self.device)
            .map_err(|e| Error::Model(format!("Failed to create tensor: {e}")))
    }

    pub fn get_provider(&self) -> Option<&Box<dyn CompletionProvider>> {
        self.provider.as_ref()
    }

    pub fn get_provider_mut(&mut self) -> Option<&mut Box<dyn CompletionProvider>> {
        self.provider.as_mut()
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
        self.provider.is_some()
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
        self.eos_token_ids.first().copied()
    }

    pub fn eos_token_ids(&self) -> &[u32] {
        &self.eos_token_ids
    }

    pub fn context_length(&self) -> usize {
        // Get context length from config if available
        self.config
            .as_ref()
            .map(|c| c.max_position_embeddings)
            .unwrap_or(2048) // Default to 2048 if not found
    }

    /// Try to download tokenizer for known models
    fn try_download_tokenizer(&mut self) -> bool {
        // Check if we can find model spec with base_repo_id
        let manifest = match super::download_manager::ModelManifest::load() {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!("Failed to load model manifest: {}", e);
                return false;
            }
        };

        let spec = match manifest.find(&self.model_name) {
            Some(s) if s.base_repo_id.is_some() => s,
            _ => {
                tracing::debug!("No base_repo_id found for model {}", self.model_name);
                return false;
            }
        };

        let base_repo_id = spec.base_repo_id.as_ref().unwrap();
        tracing::info!(
            "Attempting to download tokenizer from base repository: {}",
            base_repo_id
        );

        // Check if we're already in a tokio runtime
        let result = if tokio::runtime::Handle::try_current().is_ok() {
            // We're in an async context, spawn a new thread to avoid nested runtime
            let base_repo_id = base_repo_id.to_string();
            let model_path = self.model_path.clone();
            let tokenizer_type = spec.tokenizer_type.clone();

            // Use a separate thread to avoid runtime conflicts
            std::thread::spawn(move || {
                // Create a new runtime in the spawned thread - this is safe
                let runtime = match tokio::runtime::Runtime::new() {
                    Ok(rt) => rt,
                    Err(e) => {
                        tracing::error!("Failed to create runtime for tokenizer download: {}", e);
                        return false;
                    }
                };

                // Allow block_on here since we're in a separate thread
                #[allow(clippy::disallowed_methods)]
                runtime.block_on(async move {
                    let downloader = match super::download_manager::ModelDownloader::new() {
                        Ok(d) => d,
                        Err(e) => {
                            tracing::error!("Failed to create downloader: {}", e);
                            return false;
                        }
                    };

                    if let Some(model_path) = &model_path {
                        let gguf_path = Path::new(model_path);
                        if let Err(e) = downloader
                            .download_tokenizer(&base_repo_id, gguf_path, tokenizer_type.as_deref())
                            .await
                        {
                            tracing::error!("Failed to download tokenizer: {}", e);
                            return false;
                        }
                    }
                    true
                })
            })
            .join()
            .unwrap_or(false)
        } else {
            // Not in async context, create a runtime
            let runtime = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!("Failed to create runtime for tokenizer download: {}", e);
                    return false;
                }
            };

            // Allow block_on here since we're not in an async context
            #[allow(clippy::disallowed_methods)]
            runtime.block_on(async {
                let downloader = match super::download_manager::ModelDownloader::new() {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::error!("Failed to create downloader: {}", e);
                        return false;
                    }
                };

                if let Some(model_path) = &self.model_path {
                    let gguf_path = Path::new(model_path);
                    if let Err(e) = downloader
                        .download_tokenizer(base_repo_id, gguf_path, spec.tokenizer_type.as_deref())
                        .await
                    {
                        tracing::error!("Failed to download tokenizer: {}", e);
                        return false;
                    }
                }
                true
            })
        };

        if result {
            // Try loading the tokenizer again after download
            self.try_load_alongside_model()
        } else {
            false
        }
    }
}
