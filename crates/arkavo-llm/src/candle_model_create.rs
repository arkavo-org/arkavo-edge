use anyhow::{Result, anyhow};
use candle_core::{Tensor, Device, DType};
use std::path::Path;
use crate::Qwen3Config;
use crate::candle_model_core::CandleQwen3Model;
use crate::candle_transformer_layer::TransformerLayer;
use crate::utils;

impl CandleQwen3Model {
    /// Creates a new CandleQwen3Model with empty tensors
    pub fn new(config: &Qwen3Config) -> Result<Self> {
        let model_path = Path::new(&config.model_path);
        if !model_path.exists() && !config.model_path.starts_with("memory://") {
            return Err(anyhow!("Model path does not exist: {}", config.model_path));
        }
        
        let device = if config.use_gpu {
            if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
                match Device::new_metal(0) {
                    Ok(dev) => dev,
                    Err(_) => Device::Cpu
                }
            } else {
                Device::Cpu
            }
        } else {
            Device::Cpu
        };
        
        use crate::utils::EMBEDDED_CONFIG_JSON;
        
        let config_str = std::str::from_utf8(EMBEDDED_CONFIG_JSON)
            .map_err(|e| anyhow!("Failed to decode embedded config JSON: {}", e))?;
            
        let config_json: serde_json::Value = serde_json::from_str(config_str)
            .map_err(|e| anyhow!("Failed to parse embedded config JSON: {}", e))?;
            
        let hidden_dim = config_json["hidden_size"]
            .as_u64()
            .unwrap_or(1024) as usize;
            
        let num_layers = config_json["num_hidden_layers"]
            .as_u64()
            .unwrap_or(28) as usize;
            
        let num_heads = config_json["num_attention_heads"]
            .as_u64()
            .unwrap_or(16) as usize;
            
        let _num_kv_heads = config_json["num_key_value_heads"]
            .as_u64()
            .unwrap_or(num_heads as u64) as usize;
            
        let vocab_size = config_json["vocab_size"]
            .as_u64()
            .unwrap_or(151936) as usize;
            
        let head_dim = hidden_dim / num_heads;
        
        let embedding = Tensor::zeros((vocab_size, hidden_dim), DType::F32, &device)?;
        let position_embedding = Tensor::zeros((2048, hidden_dim), DType::F32, &device)?;
        let final_norm_weight = Tensor::ones(hidden_dim, DType::F32, &device)?;
        let final_norm_bias = Some(Tensor::zeros(hidden_dim, DType::F32, &device)?);
        let lm_head = Tensor::zeros((vocab_size, hidden_dim), DType::F32, &device)?;
        
        let mut layers = Vec::with_capacity(num_layers);
        for _ in 0..num_layers {
            let layer_dim = hidden_dim;
            let ff_dim = layer_dim * 4;
            
            let layer = TransformerLayer::new(
                Tensor::zeros((hidden_dim, hidden_dim), DType::F32, &device)?,
                Tensor::zeros((hidden_dim, hidden_dim), DType::F32, &device)?,
                Tensor::zeros((hidden_dim, hidden_dim), DType::F32, &device)?,
                Tensor::zeros((hidden_dim, hidden_dim), DType::F32, &device)?,
                Tensor::ones(hidden_dim, DType::F32, &device)?,
                Some(Tensor::zeros(hidden_dim, DType::F32, &device)?),
                Tensor::zeros((hidden_dim, ff_dim), DType::F32, &device)?,
                Some(Tensor::zeros(ff_dim, DType::F32, &device)?),
                Some(Tensor::zeros((hidden_dim, ff_dim), DType::F32, &device)?), 
                Some(Tensor::zeros(ff_dim, DType::F32, &device)?),               
                Tensor::zeros((ff_dim, hidden_dim), DType::F32, &device)?,
                Some(Tensor::zeros(hidden_dim, DType::F32, &device)?),
                Tensor::ones(hidden_dim, DType::F32, &device)?,
                Some(Tensor::zeros(hidden_dim, DType::F32, &device)?),
            );
            
            layers.push(layer);
        }
        
        Ok(Self {
            is_loaded: true,
            hidden_dim,
            num_layers,
            num_heads,
            head_dim,
            vocab_size,
            temperature: config.temperature,
            device,
            embedding,
            position_embedding,
            layers,
            final_norm_weight,
            final_norm_bias,
            lm_head,
        })
    }
    
    /// Creates a new CandleQwen3Model from the embedded model data
    pub fn new_from_embedded(config: &Qwen3Config) -> Result<Self> {
        // Determine the device based on configuration
        // For F16 models, Metal acceleration should work well
        let device = if config.use_gpu {
            if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
                match Device::new_metal(0) {
                    Ok(dev) => {
                        println!("Using Metal acceleration for GGUF model");
                        dev
                    },
                    Err(e) => {
                        println!("Failed to initialize Metal: {}, falling back to CPU", e);
                        Device::Cpu
                    }
                }
            } else {
                println!("GPU acceleration requested but not available on this platform, using CPU");
                Device::Cpu
            }
        } else {
            println!("Using CPU for GGUF model as requested");
            Device::Cpu
        };
        
        // Parse config file to get model architecture parameters
        let config_str = std::str::from_utf8(utils::EMBEDDED_CONFIG_JSON)
            .map_err(|e| anyhow!("Failed to decode config JSON: {}", e))?;
        
        let config_json: serde_json::Value = serde_json::from_str(config_str)
            .map_err(|e| anyhow!("Failed to parse config JSON: {}", e))?;
        
        // Check if we have empty embedded model data and need to fall back to default parameters
        #[allow(clippy::len_zero)]
        // We're using EMBEDDED_MODEL from embedded_model.rs directly,
        // so we shouldn't need a fallback since we're always embedding the model
        let use_fallback = false;
        
        // Extract model architecture parameters with many possible key names
        let hidden_dim = if use_fallback {
            // Fallback - Qwen3-0.6B default
            1024
        } else {
            config_json["hidden_size"]
                .as_u64()
                .or_else(|| config_json["n_embd"].as_u64())
                .or_else(|| config_json["d_model"].as_u64())
                .or_else(|| config_json["model_dim"].as_u64())
                .unwrap_or(1024) as usize
        };
            
        let num_layers = if use_fallback {
            // Fallback - Qwen3-0.6B default
            28 
        } else {
            config_json["num_hidden_layers"]
                .as_u64()
                .or_else(|| config_json["n_layer"].as_u64())
                .or_else(|| config_json["num_layers"].as_u64())
                .or_else(|| config_json["n_layers"].as_u64())
                .unwrap_or(28) as usize
        };
            
        let num_heads = if use_fallback {
            // Fallback - Qwen3-0.6B default
            16
        } else {
            config_json["num_attention_heads"]
                .as_u64()
                .or_else(|| config_json["n_head"].as_u64())
                .or_else(|| config_json["num_heads"].as_u64())
                .unwrap_or(16) as usize
        };
            
        let vocab_size = if use_fallback {
            // Fallback - Qwen3-0.6B default
            151936
        } else {
            config_json["vocab_size"]
                .as_u64()
                .or_else(|| config_json["n_vocab"].as_u64())
                .unwrap_or(151936) as usize
        };
            
        let head_dim = hidden_dim / num_heads;
        
        // Always use embedded GGUF model - it's directly included in the binary
        // via include_bytes! in embedded_model.rs
        Self::load_from_embedded_gguf(
            hidden_dim, num_layers, num_heads, head_dim, vocab_size,
            config.temperature, &device)
    }
}