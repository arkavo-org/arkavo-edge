#![allow(clippy::useless_format)]

use anyhow::{Result, anyhow};
use candle_core::{Tensor, Device, DType};
use std::path::Path;
use crate::Qwen3Config;
use crate::candle::TransformerLayer;
use crate::utils;

pub struct CandleQwen3Model {
    pub(crate) is_loaded: bool,
    
    pub(crate) hidden_dim: usize,
    pub(crate) num_layers: usize,
    pub(crate) num_heads: usize,
    pub(crate) head_dim: usize,
    pub(crate) vocab_size: usize,
    
    pub(crate) temperature: f32,
    
    pub(crate) device: Device,
    
    pub(crate) embedding: Tensor,
    pub(crate) position_embedding: Tensor,
    pub(crate) layers: Vec<TransformerLayer>,
    pub(crate) final_norm_weight: Tensor,
    pub(crate) final_norm_bias: Option<Tensor>,
    pub(crate) lm_head: Tensor,
}

impl CandleQwen3Model {
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

    // Helper method to load from embedded GGUF bytes
    fn load_from_embedded_gguf(
        hidden_dim: usize,
        num_layers: usize,
        num_heads: usize,
        head_dim: usize,
        vocab_size: usize,
        temperature: f32,
        device: &Device,
    ) -> Result<Self> {
        use std::io::Cursor;
        use candle_core::quantized::gguf_file;
        
        println!("Loading embedded GGUF model with size: {} bytes", crate::EMBEDDED_MODEL.len());
        
        // Check if we're using GPU acceleration
        let using_gpu = matches!(device, Device::Metal(_) | Device::Cuda(_));
        
        // For GGUF loading, we need to use CPU initially (quantized tensors must be loaded on CPU)
        // After dequantization, we'll move tensors to the GPU if requested
        let loading_device = Device::Cpu;
        
        if using_gpu {
            println!("Will load GGUF tensors on CPU first, then transfer to {}", 
                     if matches!(device, Device::Metal(_)) { "Metal GPU" } else { "CUDA GPU" });
        }
        
        // Create a Cursor to read from the embedded bytes as if it were a file
        let mut gguf_data = Cursor::new(crate::EMBEDDED_MODEL);
        
        // Parse the GGUF format header and metadata
        let gguf_content = match gguf_file::Content::read(&mut gguf_data) {
            Ok(content) => {
                println!("Successfully parsed GGUF header with {} tensors", content.tensor_infos.len());
                content
            },
            Err(e) => {
                println!("Failed to parse GGUF header: {}", e);
                return Err(anyhow!("Failed to parse GGUF header: {}", e));
            }
        };
        
        // Extract tensors from the GGUF file
        let mut model_tensors = std::collections::HashMap::new();
        
        // Debug: print available tensor names from GGUF
        // println!("Available tensors in GGUF model:");
        // for (name, _) in gguf_content.tensor_infos.iter() {
        //     println!("   - {}", name);
        // }
        
        // Get the tensor metadata and store the tensors
        // We need to seek and read the tensors directly from the file
        for (tensor_name, _tensor_info) in gguf_content.tensor_infos.iter() {
            // Need to create a new cursor for each tensor read operation
            let mut tensor_reader = Cursor::new(crate::EMBEDDED_MODEL);
            
            // Try to load the tensor from GGUF file (always load on CPU first)
            match gguf_content.tensor(&mut tensor_reader, tensor_name, &loading_device) {
                Ok(q_tensor) => {
                    // Convert the quantized tensor to a regular tensor on CPU
                    match q_tensor.dequantize(&loading_device) {
                        Ok(t) => {
                            // If using GPU, transfer the tensor to GPU after dequantization
                            if using_gpu {
                                match t.to_device(device) {
                                    Ok(gpu_tensor) => {
                                        model_tensors.insert(tensor_name.clone(), gpu_tensor);
                                    },
                                    Err(e) => {
                                        println!("Warning: Failed to move tensor {} to GPU: {}", tensor_name, e);
                                        // Fall back to using the CPU tensor if we can't move to GPU
                                        model_tensors.insert(tensor_name.clone(), t);
                                    }
                                }
                            } else {
                                // Keep tensor on CPU
                                model_tensors.insert(tensor_name.clone(), t);
                            }
                        },
                        Err(e) => {
                            return Err(anyhow!("Failed to dequantize tensor {}: {}", tensor_name, e));
                        }
                    }
                },
                Err(e) => {
                    return Err(anyhow!("Failed to load tensor {}: {}", tensor_name, e));
                }
            }
        }
        
        // Create the model structure from the GGUF tensors
        // The exact tensor names depend on the specific GGUF model format
        
        // Extract the token embeddings
        let embedding = model_tensors
            .get("token_embd.weight")  // Standard GGUF name (Qwen3 uses this)
            .or_else(|| model_tensors.get("model.embed_tokens.weight"))
            .or_else(|| model_tensors.get("embedding.weight"))
            .ok_or_else(|| anyhow!("Token embedding matrix not found in GGUF model"))?
            .clone();
        
        // Extract position embeddings (only for models that use them)
        // Modern models like Qwen3 may not use explicit position embeddings
        let position_embedding = model_tensors
            .get("position_embd.weight")
            .or_else(|| model_tensors.get("model.embed_positions.weight"))
            // For Qwen3, use a zero tensor since it uses rotary embeddings instead of positional
            .cloned()
            .unwrap_or_else(|| {
                println!("Position embeddings not found, creating zero position embeddings");
                // Create a default position embedding if not found - Qwen3 uses RoPE instead of position embeddings
                // The zero tensor won't affect the actual positional encoding which happens in attention
                Tensor::zeros((2048, hidden_dim), DType::F32, device)
                    .expect("Failed to create default position embeddings")
            });
        
        // Extract final layer norm weights or create default
        let final_norm_weight = model_tensors
            .get("model.final_layernorm.weight")
            .or_else(|| model_tensors.get("model.norm.weight"))
            .or_else(|| model_tensors.get("ln_f.weight"))
            .or_else(|| model_tensors.get("norm.weight"))
            .or_else(|| model_tensors.get("output_norm.weight"))  // Qwen3 GGUF format
            .cloned()
            .unwrap_or_else(|| {
                println!("Final layer norm weights not found, using default ones");
                Tensor::ones(hidden_dim, DType::F32, device)
                    .expect("Failed to create default final norm weights")
            });
        
        // Extract final layer norm bias if available
        let final_norm_bias = model_tensors
            .get("model.final_layernorm.bias")
            .or_else(|| model_tensors.get("model.norm.bias"))
            .or_else(|| model_tensors.get("ln_f.bias"))
            .or_else(|| model_tensors.get("output_norm.bias"))  // Qwen3 GGUF format
            .cloned();
        
        // Extract LM head weights
        let lm_head = model_tensors
            .get("lm_head.weight")
            .or_else(|| model_tensors.get("output.weight"))  // Qwen3 GGUF uses this
            .or_else(|| model_tensors.get("head.weight"))
            .cloned()
            .unwrap_or_else(|| {
                // Some models tie weights with the embedding (weight tying is common)
                // Qwen3 has an output.weight but we'll fall back to token embeddings if needed
                if model_tensors.contains_key("token_embd.weight") {
                    embedding.clone()
                } else {
                    // Create a fallback lm_head with zeros since we couldn't find one
                    Tensor::zeros((vocab_size, hidden_dim), DType::F32, device)
                        .expect("Failed to create fallback LM head weights")
                }
            });
        
        // Create transformer layers
        let mut layers = Vec::with_capacity(num_layers);
        for layer_idx in 0..num_layers {
            // Common layer prefix patterns in GGUF models
            let layer_prefixes = [
                format!("model.layers.{}", layer_idx),
                format!("transformer.h.{}", layer_idx),
                format!("layers.{}", layer_idx),
                format!("h.{}", layer_idx),
                format!("blk.{}", layer_idx),  // Qwen3 GGUF format
            ];
            
            // Find the right prefix for this model
            let mut found_prefix = None;
            for prefix in &layer_prefixes {
                // Common key patterns for different model formats
                let test_keys = [
                    format!("{}.self_attn.q_proj.weight", prefix),
                    format!("{}.attention.wq.weight", prefix),
                    format!("{}.attn_q.weight", prefix),  // Qwen3 GGUF format
                ];
                
                for test_key in &test_keys {
                    if model_tensors.contains_key(test_key) {
                        found_prefix = Some(prefix);
                        break;
                    }
                }
                
                if found_prefix.is_some() {
                    break;
                }
            }
            
            let prefix = found_prefix.ok_or_else(|| {
                anyhow!("Could not find transformer layer {} in GGUF model", layer_idx)
            })?;
            
            // Helper function to get tensor with different possible names
            let get_tensor = |base_name: &str, alternatives: &[&str]| -> Result<Tensor> {
                let mut full_names = vec![format!("{}.{}", prefix, base_name)];
                for alt in alternatives {
                    full_names.push(format!("{}.{}", prefix, alt));
                }
                
                for name in full_names {
                    if let Some(tensor) = model_tensors.get(&name) {
                        return Ok(tensor.clone());
                    }
                }
                
                // If not found, create a default tensor
                let key_parts: Vec<&str> = base_name.split('.').collect();
                if key_parts.len() >= 2 && key_parts[1].contains("weight") {
                    if key_parts[0].contains("q_proj") || key_parts[0].contains("k_proj") ||
                       key_parts[0].contains("v_proj") || key_parts[0].contains("o_proj") {
                        return Ok(Tensor::zeros((hidden_dim, hidden_dim), DType::F32, device)?);
                    } else if key_parts[0].contains("gate_proj") || key_parts[0].contains("up_proj") {
                        let ff_dim = 4 * hidden_dim;
                        return Ok(Tensor::zeros((hidden_dim, ff_dim), DType::F32, device)?);
                    } else if key_parts[0].contains("down_proj") {
                        let ff_dim = 4 * hidden_dim;
                        return Ok(Tensor::zeros((ff_dim, hidden_dim), DType::F32, device)?);
                    }
                } else if key_parts.len() >= 2 && key_parts[1].contains("bias") && 
                          (key_parts[0].contains("q_proj") || key_parts[0].contains("k_proj") ||
                           key_parts[0].contains("v_proj") || key_parts[0].contains("o_proj") ||
                           key_parts[0].contains("input_layernorm")) {
                    return Ok(Tensor::zeros(hidden_dim, DType::F32, device)?);
                }
                
                Err(anyhow!("Could not find tensor for {} in GGUF model", base_name))
            };
            
            // Get all tensors for the transformer layer
            let q_weight = get_tensor("self_attn.q_proj.weight", 
                &["attention.wq.weight", "attn.q_proj.weight", "attn_q.weight"])?;
                
            let k_weight = get_tensor("self_attn.k_proj.weight", 
                &["attention.wk.weight", "attn.k_proj.weight", "attn_k.weight"])?;
                
            let v_weight = get_tensor("self_attn.v_proj.weight", 
                &["attention.wv.weight", "attn.v_proj.weight", "attn_v.weight"])?;
                
            let o_weight = get_tensor("self_attn.o_proj.weight", 
                &["attention.wo.weight", "attn.c_proj.weight", "attn_output.weight"])?;
                
            let input_ln_weight = get_tensor("input_layernorm.weight", 
                &["ln_1.weight", "attention_norm.weight", "attn_norm.weight"])?;
                
            let input_ln_bias = model_tensors
                .get(&format!("{}.input_layernorm.bias", prefix))
                .or_else(|| model_tensors.get(&format!("{}.ln_1.bias", prefix)))
                .or_else(|| model_tensors.get(&format!("{}.attention_norm.bias", prefix)))
                .or_else(|| model_tensors.get(&format!("{}.attn_norm.bias", prefix)))
                .cloned();
            
            let gate_proj = get_tensor("mlp.gate_proj.weight", 
                &["mlp.c_fc.weight", "feed_forward.w1.weight", "ffn_gate.weight"])?;
                
            let gate_bias = model_tensors
                .get(&format!("{}.mlp.gate_proj.bias", prefix))
                .or_else(|| model_tensors.get(&format!("{}.mlp.c_fc.bias", prefix)))
                .or_else(|| model_tensors.get(&format!("{}.ffn_gate.bias", prefix)))
                .cloned();
                
            // For Qwen3, up_proj corresponds to ffn_up
            let up_proj = model_tensors
                .get(&format!("{}.mlp.up_proj.weight", prefix))
                .or_else(|| model_tensors.get(&format!("{}.mlp.w1.weight", prefix)))
                .or_else(|| model_tensors.get(&format!("{}.ffn_up.weight", prefix)))
                .cloned();
                
            let up_bias = model_tensors
                .get(&format!("{}.mlp.up_proj.bias", prefix))
                .or_else(|| model_tensors.get(&format!("{}.mlp.w1.bias", prefix)))
                .or_else(|| model_tensors.get(&format!("{}.ffn_up.bias", prefix)))
                .cloned();
                
            let down_proj = get_tensor("mlp.down_proj.weight", 
                &["mlp.c_proj.weight", "feed_forward.w2.weight", "ffn_down.weight"])?;
                
            let down_bias = model_tensors
                .get(&format!("{}.mlp.down_proj.bias", prefix))
                .or_else(|| model_tensors.get(&format!("{}.mlp.c_proj.bias", prefix)))
                .or_else(|| model_tensors.get(&format!("{}.ffn_down.bias", prefix)))
                .cloned();
                
            let post_ln_weight = get_tensor("post_attention_layernorm.weight", 
                &["ln_2.weight", "ffn_norm.weight"])?;
                
            let post_ln_bias = model_tensors
                .get(&format!("{}.post_attention_layernorm.bias", prefix))
                .or_else(|| model_tensors.get(&format!("{}.ln_2.bias", prefix)))
                .or_else(|| model_tensors.get(&format!("{}.ffn_norm.bias", prefix)))
                .cloned();
            
            // Create the transformer layer
            let layer = TransformerLayer::new(
                q_weight,
                k_weight,
                v_weight,
                o_weight,
                input_ln_weight,
                input_ln_bias,
                gate_proj,
                gate_bias,
                up_proj,
                up_bias,
                down_proj,
                down_bias,
                post_ln_weight,
                post_ln_bias,
            );
            
            layers.push(layer);
        }
        
        // Return the complete model structure
        Ok(Self {
            is_loaded: true,
            hidden_dim,
            num_layers,
            num_heads,
            head_dim,
            vocab_size,
            temperature,
            device: device.clone(),
            embedding,
            position_embedding,
            layers,
            final_norm_weight,
            final_norm_bias,
            lm_head,
        })
    }
    
    pub fn is_using_gpu(&self) -> bool {
        matches!(self.device, Device::Cuda(_) | Device::Metal(_))
    }
    
    pub fn get_acceleration_name(&self) -> &'static str {
        match self.device {
            Device::Cuda(_) => "CUDA (NVIDIA GPU)",
            Device::Metal(_) => "Metal (Apple GPU)",
            _ => "CPU",
        }
    }
}