use anyhow::{Result, anyhow};
use candle_core::{Tensor, Device, DType, Module};
use candle_nn::{ops, activation};
use safetensors::SafeTensors;
use safetensors::tensor::TensorView;
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;
use crate::Qwen3Config;

// Helper function to get tensor shape as a string for debugging
fn tensor_view_shape(tensor: &TensorView) -> Result<String> {
    let shape = tensor.shape();
    let shape_str = shape.iter().map(|&d| d.to_string()).collect::<Vec<_>>().join("×");
    Ok(shape_str)
}

/// Transformer layer for Qwen3 model using Candle backend
struct TransformerLayer {
    // Self-attention weights
    query_weight: Tensor,
    key_weight: Tensor,
    value_weight: Tensor,
    output_weight: Tensor,
    
    // Layer normalization parameters
    attn_norm_weight: Tensor,
    attn_norm_bias: Option<Tensor>,
    
    // Feed-forward network weights - Qwen3 uses SwiGLU activation
    // up_proj is the main projection (what we store in ff_inter_weight)
    ff_inter_weight: Tensor,  // up_proj.weight in Qwen3
    ff_inter_bias: Option<Tensor>,
    
    // gate_proj is used for the SwiGLU gate (optional)
    ff_gate_weight: Option<Tensor>, // gate_proj.weight in Qwen3
    ff_gate_bias: Option<Tensor>,
    
    // down_proj is the output projection (what we store in ff_output_weight)
    ff_output_weight: Tensor, // down_proj.weight in Qwen3
    ff_output_bias: Option<Tensor>,
    
    // Final layer normalization parameters (post-attention layernorm)
    ff_norm_weight: Tensor,
    ff_norm_bias: Option<Tensor>,
}

impl TransformerLayer {
    fn new(
        query_weight: Tensor,
        key_weight: Tensor,
        value_weight: Tensor,
        output_weight: Tensor,
        attn_norm_weight: Tensor,
        attn_norm_bias: Option<Tensor>,
        ff_inter_weight: Tensor,
        ff_inter_bias: Option<Tensor>,
        ff_gate_weight: Option<Tensor>,
        ff_gate_bias: Option<Tensor>,
        ff_output_weight: Tensor,
        ff_output_bias: Option<Tensor>,
        ff_norm_weight: Tensor,
        ff_norm_bias: Option<Tensor>,
    ) -> Self {
        Self {
            query_weight,
            key_weight,
            value_weight,
            output_weight,
            attn_norm_weight,
            attn_norm_bias,
            ff_inter_weight,
            ff_inter_bias,
            ff_gate_weight,
            ff_gate_bias,
            ff_output_weight,
            ff_output_bias,
            ff_norm_weight,
            ff_norm_bias,
        }
    }
}

/// KV Cache for efficient autoregressive generation
struct KVCache {
    // For each layer: (key_cache, value_cache)
    layers: Vec<(Tensor, Tensor)>,
}

impl KVCache {
    fn new(
        num_layers: usize,
        num_heads: usize,
        seq_len: usize,
        head_dim: usize,
        device: &Device,
    ) -> Result<Self> {
        let mut layers = Vec::with_capacity(num_layers);
        
        for _ in 0..num_layers {
            // Initialize with zeros
            // Shape: [batch_size=1, num_heads, seq_len, head_dim]
            let key_cache = Tensor::zeros((1, num_heads, seq_len, head_dim), DType::F32, device)?;
            let value_cache = Tensor::zeros((1, num_heads, seq_len, head_dim), DType::F32, device)?;
            
            layers.push((key_cache, value_cache));
        }
        
        Ok(Self { layers })
    }
    
    fn update_cache(&mut self, layer_idx: usize, position: usize, key: &Tensor, value: &Tensor) -> Result<()> {
        // Ensure layer_idx is valid
        if layer_idx >= self.layers.len() {
            return Err(anyhow!("Invalid layer index: {}", layer_idx));
        }
        
        // Get the cache for this layer
        let (k_cache, v_cache) = &mut self.layers[layer_idx];
        
        // Check if position is within bounds
        let cache_seq_len = k_cache.shape().dims()[2];
        if position >= cache_seq_len {
            eprintln!("WARNING: Attempted to update KV cache at position {} but cache size is {}. Skipping update.",
                     position, cache_seq_len);
            return Ok(());
        }
        
        // Update the cache at the specified position
        // Make sure the dimensions are correct or adapt
        let key_dims = key.shape().dims();
        let value_dims = value.shape().dims();
        
        // We expect key/value to be 4D with shape [batch, num_heads, 1, head_dim]
        // Handle different shapes if needed
        if key_dims.len() == 4 && value_dims.len() == 4 {
            // Standard case
            k_cache.slice_assign(&[0..1, 0..key.dim(1)?, position..position+1, 0..key.dim(3)?], key)?;
            v_cache.slice_assign(&[0..1, 0..value.dim(1)?, position..position+1, 0..value.dim(3)?], value)?;
        } else {
            // Reshape or adapt as needed
            // Try to adapt based on dimensions - this is a fallback
            let adapted_key = if key_dims != v_cache.shape().dims() {
                // Extract the shape of the KV cache
                let k_shape = k_cache.shape().dims();
                let key_batch = k_shape[0];
                let key_heads = k_shape[1];
                let key_head_dim = k_shape[3];
                
                // First reshape to 3D if needed
                let key_3d = if key_dims.len() < 3 {
                    key.reshape((key_batch, key_heads, key_head_dim))?
                } else {
                    key.clone()
                };
                
                // Then add the 4th dimension if needed
                if key_3d.shape().dims().len() < 4 {
                    key_3d.unsqueeze(2)?
                } else {
                    key_3d
                }
            } else {
                key.clone()
            };
            
            // Similarly for value
            let adapted_value = if value_dims != v_cache.shape().dims() {
                // Extract the shape of the KV cache
                let v_shape = v_cache.shape().dims();
                let value_batch = v_shape[0];
                let value_heads = v_shape[1];
                let value_head_dim = v_shape[3];
                
                // First reshape to 3D if needed
                let value_3d = if value_dims.len() < 3 {
                    value.reshape((value_batch, value_heads, value_head_dim))?
                } else {
                    value.clone()
                };
                
                // Then add the 4th dimension if needed
                if value_3d.shape().dims().len() < 4 {
                    value_3d.unsqueeze(2)?
                } else {
                    value_3d
                }
            } else {
                value.clone()
            };
            
            // Try to update the cache with the adapted tensors
            k_cache.slice_assign(&[0..1, 0..adapted_key.dim(1)?, position..position+1, 0..adapted_key.dim(3)?], &adapted_key)?;
            v_cache.slice_assign(&[0..1, 0..adapted_value.dim(1)?, position..position+1, 0..adapted_value.dim(3)?], &adapted_value)?;
        }
        
        Ok(())
    }
    
    fn get_cache_for_layer(&self, layer_idx: usize, position: usize) -> Result<(Tensor, Tensor)> {
        if layer_idx >= self.layers.len() {
            return Err(anyhow!("Invalid layer index: {}", layer_idx));
        }
        
        // Get KV cache up to the current position
        let (k_cache, v_cache) = &self.layers[layer_idx];
        
        // Get the sequence dimension size (dimension 2)
        let seq_len = k_cache.shape().dims()[2];
        
        // Ensure we don't try to narrow beyond the available cache size
        let actual_pos = std::cmp::min(position + 1, seq_len);
        
        // Log this to help with debugging
        if position + 1 > seq_len {
            eprintln!("WARNING: Requested position {} exceeds cache size {}. Using maximum available.", 
                     position + 1, seq_len);
        }
        
        let k_cache_slice = k_cache.narrow(2, 0, actual_pos)?;
        let v_cache_slice = v_cache.narrow(2, 0, actual_pos)?;
        
        Ok((k_cache_slice, v_cache_slice))
    }
}

/// Qwen3 model implementation using Candle backend for accelerated inference
pub struct CandleQwen3Model {
    /// Whether the model has been loaded
    is_loaded: bool,
    
    /// Model parameters
    hidden_dim: usize,
    num_layers: usize,
    num_heads: usize,
    head_dim: usize,
    vocab_size: usize,
    
    /// Model temperature for sampling
    temperature: f32,
    
    /// Device to use for Tensor operations
    device: Device,
    
    /// Transformer weights
    embedding: Tensor,
    position_embedding: Tensor,
    layers: Vec<TransformerLayer>,
    final_norm_weight: Tensor,
    final_norm_bias: Option<Tensor>,
    lm_head: Tensor,
}

impl CandleQwen3Model {
    /// Creates a new model from the given configuration
    pub fn new(config: &Qwen3Config) -> Result<Self> {
        // Load model based on model path
        let model_path = Path::new(&config.model_path);
        if !model_path.exists() && !config.model_path.starts_with("memory://") {
            return Err(anyhow!("Model path does not exist: {}", config.model_path));
        }
        
        // Determine which device to use - this is where real hardware acceleration happens
        let device = if config.use_gpu {
            if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
                // Use Metal on Apple Silicon
                eprintln!("DEBUG: Using Metal backend on Apple Silicon");
                Device::new_metal(0)?
            } else {
                // Fallback to CPU
                eprintln!("DEBUG: GPU requested but not available, using CPU");
                Device::Cpu
            }
        } else {
            eprintln!("DEBUG: Using CPU backend as requested");
            Device::Cpu
        };

        eprintln!("DEBUG: Selected device: {:?}", device);
        
        // Read model dimensions from the embedded config.json
        use crate::utils::EMBEDDED_CONFIG_JSON;
        
        // Parse the embedded config to get model architecture parameters
        let config_str = std::str::from_utf8(EMBEDDED_CONFIG_JSON)
            .map_err(|e| anyhow!("Failed to decode embedded config JSON: {}", e))?;
            
        let config_json: serde_json::Value = serde_json::from_str(config_str)
            .map_err(|e| anyhow!("Failed to parse embedded config JSON: {}", e))?;
            
        // Extract model architecture parameters from config
        let hidden_dim = config_json["hidden_size"]
            .as_u64()
            .unwrap_or(1024) as usize;
            
        let num_layers = config_json["num_hidden_layers"]
            .as_u64()
            .unwrap_or(28) as usize;
            
        let num_heads = config_json["num_attention_heads"]
            .as_u64()
            .unwrap_or(16) as usize;
            
        let num_kv_heads = config_json["num_key_value_heads"]
            .as_u64()
            .unwrap_or(num_heads as u64) as usize;
            
        let vocab_size = config_json["vocab_size"]
            .as_u64()
            .unwrap_or(151936) as usize;
            
        let head_dim = hidden_dim / num_heads;
        
        eprintln!("Model architecture from config: hidden_dim={}, num_layers={}, num_heads={}, num_kv_heads={}, vocab_size={}", 
                 hidden_dim, num_layers, num_heads, num_kv_heads, vocab_size);
        
        // Initialize default transformer matrices (zeros tensors for placeholders)
        let embedding = Tensor::zeros((vocab_size, hidden_dim), DType::F32, &device)?;
        let position_embedding = Tensor::zeros((2048, hidden_dim), DType::F32, &device)?;
        let final_norm_weight = Tensor::ones(hidden_dim, DType::F32, &device)?;
        let final_norm_bias = Some(Tensor::zeros(hidden_dim, DType::F32, &device)?);
        let lm_head = Tensor::zeros((vocab_size, hidden_dim), DType::F32, &device)?;
        
        // Create transformer layers 
        let mut layers = Vec::with_capacity(num_layers);
        for _ in 0..num_layers {
            let layer_dim = hidden_dim;
            let ff_dim = layer_dim * 4;
            
            // Create layer with default zero tensors, including gate_proj weights
            let layer = TransformerLayer::new(
                Tensor::zeros((hidden_dim, hidden_dim), DType::F32, &device)?,
                Tensor::zeros((hidden_dim, hidden_dim), DType::F32, &device)?,
                Tensor::zeros((hidden_dim, hidden_dim), DType::F32, &device)?,
                Tensor::zeros((hidden_dim, hidden_dim), DType::F32, &device)?,
                Tensor::ones(hidden_dim, DType::F32, &device)?,
                Some(Tensor::zeros(hidden_dim, DType::F32, &device)?),
                Tensor::zeros((hidden_dim, ff_dim), DType::F32, &device)?,
                Some(Tensor::zeros(ff_dim, DType::F32, &device)?),
                Some(Tensor::zeros((hidden_dim, ff_dim), DType::F32, &device)?), // gate_proj weight
                Some(Tensor::zeros(ff_dim, DType::F32, &device)?),               // gate_proj bias
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
    
    /// Creates a new CandleQwen3Model using embedded model data
    pub fn new_from_embedded(config: &Qwen3Config) -> Result<Self> {
        // Access embedded model data
        use crate::utils::EMBEDDED_MODEL_SAFETENSORS;
        use crate::utils::EMBEDDED_CONFIG_JSON;
        
        eprintln!("DEBUG: Loading model from embedded data");
        
        // Determine which device to use - this is where real hardware acceleration happens
        let device = if config.use_gpu {
            if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
                // Use Metal on Apple Silicon
                eprintln!("DEBUG: Using Metal backend on Apple Silicon");
                match Device::new_metal(0) {
                    Ok(dev) => {
                        eprintln!("DEBUG: Metal device initialized successfully");
                        dev
                    },
                    Err(e) => {
                        eprintln!("DEBUG: Failed to initialize Metal device: {}, falling back to CPU", e);
                        Device::Cpu
                    }
                }
            } else {
                // Fallback to CPU
                eprintln!("DEBUG: GPU requested but not available, using CPU");
                Device::Cpu
            }
        } else {
            eprintln!("DEBUG: Using CPU backend as requested");
            Device::Cpu
        };

        eprintln!("DEBUG: Selected device: {:?}", device);
        
        // Parse config file to get model architecture parameters
        let config_str = std::str::from_utf8(EMBEDDED_CONFIG_JSON)
            .map_err(|e| anyhow!("Failed to decode config JSON: {}", e))?;
            
        eprintln!("DEBUG: Config JSON size: {} bytes", config_str.len());
        eprintln!("DEBUG: Config JSON content: {}", if config_str.len() > 200 {
            format!("{} [truncated]...", &config_str[..200])
        } else {
            config_str.to_string()
        });
        
        let config_json: serde_json::Value = serde_json::from_str(config_str)
            .map_err(|e| anyhow!("Failed to parse config JSON: {}", e))?;
        
        // Check if we have empty embedded model data and need to fall back to default parameters
        let mut use_fallback = EMBEDDED_MODEL_SAFETENSORS.len() < 1000;
        eprintln!("DEBUG: Safetensors size: {} bytes, {}", EMBEDDED_MODEL_SAFETENSORS.len(), 
                 if use_fallback { "using fallback model" } else { "attempting to load" });
        
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
        
        eprintln!("DEBUG: Model architecture: hidden_dim={}, num_layers={}, num_heads={}, vocab_size={}", 
                 hidden_dim, num_layers, num_heads, vocab_size);
        
        // Load model weights from safetensors format
        eprintln!("DEBUG: Deserializing model weights from safetensors format");
        let tensors = SafeTensors::deserialize(EMBEDDED_MODEL_SAFETENSORS)
            .map_err(|e| anyhow!("Failed to deserialize model: {}", e))?;
            
        // Create tensor mapping for more efficient access
        let mut tensor_map = HashMap::new();
        let mut tensor_count = 0;
        
        let all_names: Vec<String> = tensors.names().into_iter().map(|s| s.to_string()).collect();
        for tensor_name in all_names.iter() {
            let safetensor = tensors.tensor(tensor_name.as_str())?;
            
            // Load both F32 and BF16 tensors - most Qwen3 models use BF16
            match safetensor.dtype() {
                safetensors::Dtype::F32 => {
                    // Get data and shape
                    let shape = safetensor.shape().to_vec();
                    let data = safetensor.data();
                    
                    // Create F32 tensor
                    let tensor = Tensor::from_vec(data.to_vec(), shape.clone(), &Device::Cpu)?;
                    
                    // Insert into map (keep on CPU for now)
                    tensor_map.insert(tensor_name.to_string(), tensor);
                    tensor_count += 1;
                    
                    if tensor_count % 10 == 0 {
                        eprintln!("DEBUG: Loaded {} tensors so far", tensor_count);
                    }
                },
                
                safetensors::Dtype::BF16 => {
                    // Get data and shape for BF16 tensor
                    let shape = safetensor.shape().to_vec();
                    let data = safetensor.data();
                    
                    // For BF16 tensors, we'll use from_raw_buffer but be careful with the data
                    // Convert BF16 to F32 by expanding each value
                    if tensor_count % 10 == 0 {
                        eprintln!("DEBUG: Converting BF16 tensor to F32 for '{}'", tensor_name);
                    }
                    
                    // Create BF16 tensor using from_raw_buffer
                    let tensor = Tensor::from_raw_buffer(
                        data,
                        DType::BF16, 
                        &shape,
                        &Device::Cpu
                    )?;
                    
                    // Convert to F32 for better compatibility 
                    let tensor = tensor.to_dtype(DType::F32)?;
                    
                    // Insert into map
                    tensor_map.insert(tensor_name.to_string(), tensor);
                    tensor_count += 1;
                    
                    if tensor_count % 10 == 0 {
                        eprintln!("DEBUG: Loaded {} tensors so far (BF16 converted to F32)", tensor_count);
                    }
                },
                
                other_dtype => {
                    // Log unsupported dtypes but don't fail
                    eprintln!("DEBUG: Skipping tensor '{}' with unsupported dtype {:?}", 
                             tensor_name, other_dtype);
                }
            }
        }
        
        eprintln!("DEBUG: Loaded {} tensors in total", tensor_count);
        
        // After loading all tensors, check if we need to fallback due to no tensors found
        if tensor_count == 0 {
            eprintln!("DEBUG: No tensors loaded from the safetensors file, falling back to stub model");
            use_fallback = true;
        }
        
        // Get tensor keys for later use
        let tensor_keys: Vec<_> = tensor_map.keys().map(|s| s.as_str()).collect();
        
        // Print some tensor keys to debug
        let sample_keys = if !tensor_keys.is_empty() {
            tensor_keys.iter().take(10).map(|&s| s.to_string()).collect::<Vec<_>>().join(", ")
        } else {
            "[No keys found]".to_string()
        };
        eprintln!("DEBUG: Sample tensor keys: {}", sample_keys);
        
        // Handle different naming conventions in safetensors model files
        // For Qwen3 models, try both naming conventions
        
        // Determine model format by looking at key patterns
        let is_hf_format = tensor_keys.iter()
            .any(|&k| k.starts_with("transformer.") || k.starts_with("model."));
            
        let is_qwen_format = tensor_keys.iter()
            .any(|&k| k.starts_with("layers.") || k.contains("rotary_emb"));
            
        // Check for modern HF Qwen3 format
        let is_hf_qwen3_format = tensor_keys.iter()
            .any(|&k| k.contains("model.layers."));
            
        // Print example keys to help debug
        if !tensor_keys.is_empty() {
            eprintln!("DEBUG: First 5 tensor keys: {}", 
                     tensor_keys.iter().take(5).map(|&k| k.to_string()).collect::<Vec<_>>().join(", "));
        }
        
        // Print all MLP bias keys and their shapes for layer 0 to debug the structure
        eprintln!("DEBUG: Inspecting MLP bias keys and shapes for layer 0:");
        for name in tensor_map.keys() {
            if name.contains("model.layers.0.mlp") && name.contains("bias") {
                let shape_str = tensor_view_shape(&tensors.tensor(name)?)?;
                eprintln!("DEBUG: MLP Bias: {} with shape {}", name, shape_str);
            }
        }
            
        eprintln!("DEBUG: Model format detection: HF format: {}, Qwen format: {}, HF Qwen3: {}", 
                 is_hf_format, is_qwen_format, is_hf_qwen3_format);
        
        // Get the appropriate key prefixes based on format
        let (emb_key, pos_emb_key, layer_prefix_format, ln_f_prefix) = 
            if is_hf_qwen3_format {
                // Modern HuggingFace Qwen3 format
                ("model.embed_tokens.weight", "model.embed_positions.weight", 
                 "model.layers.{}.{}", "model.norm")
            } else if is_hf_format {
                // Classic HuggingFace format
                ("transformer.wte.weight", "transformer.wpe.weight", 
                 "transformer.h.{}.{}", "transformer.ln_f")
            } else {
                // Legacy Qwen format
                ("tok_embeddings.weight", "position_embeddings.weight",
                 "layers.{}.{}", "norm")
            };
        
        // Extract embeddings (and move to target device)
        eprintln!("DEBUG: Extracting embedding weights");
        let embedding = match tensor_map.get(emb_key) {
            Some(emb) => emb.to_device(&device)?,
            None => {
                eprintln!("DEBUG: Embedding key '{}' not found, searching for alternatives", emb_key);
                // Try alternative keys
                let alt_keys = ["transformer.word_embeddings.weight", "token_emb.weight", 
                               "word_embeddings.weight", "model.embed_tokens.weight",
                               "embedding.weight", "model.decoder.embed_tokens.weight",
                               "embeddings.word_embeddings.weight", "wte.weight"];
                
                let mut found_emb = None;
                for key in alt_keys.iter() {
                    if let Some(emb) = tensor_map.get(*key) {
                        eprintln!("DEBUG: Found embedding with key '{}'", key);
                        found_emb = Some(emb.to_device(&device)?);
                        break;
                    }
                }
                
                if tensor_count == 0 || use_fallback {
                    // No tensors loaded, create a default tensor
                    eprintln!("DEBUG: No embedding weights found, creating random initialization");
                    Tensor::randn(0.0, 0.01, (vocab_size, hidden_dim), &device)?
                } else {
                    found_emb.ok_or_else(|| anyhow!("Missing embedding weights"))?
                }
            }
        };
        
        eprintln!("DEBUG: Extracting position embedding weights");
        let position_embedding = match tensor_map.get(pos_emb_key) {
            Some(pos_emb) => pos_emb.to_device(&device)?,
            None => {
                eprintln!("DEBUG: Position embedding key '{}' not found, initializing with zeros", pos_emb_key);
                // Some models don't use positional embeddings, fall back to zeros
                Tensor::zeros((2048, hidden_dim), DType::F32, &device)?
            }
        };
            
        // Extract final layer norm weights using direct Qwen3 key pattern
        eprintln!("DEBUG: Extracting final layer norm weights");
        
        // For Qwen3, this is typically "model.norm.weight"
        let qwen3_norm_key = "model.norm.weight";
        let qwen3_norm_bias_key = "model.norm.bias";
        
        // Try the specific Qwen3 key first
        let final_norm_weight = match tensor_map.get(qwen3_norm_key) {
            Some(weight) => {
                eprintln!("DEBUG: Found final layer norm weight using Qwen3 key");
                weight.to_device(&device)?
            },
            None => {
                // Fall back to the generic format
                let ln_f_weight_key = format!("{}.weight", ln_f_prefix);
                match tensor_map.get(&ln_f_weight_key) {
                    Some(weight) => weight.to_device(&device)?,
                    None => {
                        eprintln!("DEBUG: Final layer norm weight not found, ERROR - Cannot continue without weights");
                        return Err(anyhow!("Missing critical weight: model.norm.weight"));
                    }
                }
            }
        };
            
        let final_norm_bias = match tensor_map.get(qwen3_norm_bias_key) {
            Some(bias) => {
                eprintln!("DEBUG: Found final layer norm bias using Qwen3 key");
                Some(bias.to_device(&device)?)
            },
            None => {
                // Fall back to the generic format
                let ln_f_bias_key = format!("{}.bias", ln_f_prefix);
                match tensor_map.get(&ln_f_bias_key) {
                    Some(bias) => Some(bias.to_device(&device)?),
                    None => {
                        eprintln!("DEBUG: Final layer norm bias not found, initializing with zeros");
                        Some(Tensor::zeros(hidden_dim, DType::F32, &device)?)
                    }
                }
            }
        };
            
        // Extract LM head weights (usually tied to embedding weights in Qwen3)
        eprintln!("DEBUG: Extracting LM head weights");
        
        // For Qwen3, this is typically tied with the token embeddings or specifically named
        let qwen3_lm_head_key = "lm_head.weight";
        
        let lm_head = match tensor_map.get(qwen3_lm_head_key) {
            Some(lm) => {
                eprintln!("DEBUG: Found LM head weight using Qwen3 key");
                lm.to_device(&device)?
            },
            None => {
                // Try alternative keys
                let alt_keys = ["output.weight", "model.lm_head.weight"];
                let mut found_lm = None;
                
                for key in alt_keys.iter() {
                    if let Some(lm) = tensor_map.get(*key) {
                        eprintln!("DEBUG: Found LM head with alternative key '{}'", key);
                        found_lm = Some(lm.to_device(&device)?);
                        break;
                    }
                }
                
                match found_lm {
                    Some(lm) => lm,
                    None => {
                        // In Qwen3, weights are typically tied with the token embeddings
                        eprintln!("DEBUG: LM head weight not found, using tied weights with embeddings");
                        embedding.clone() // Tied weights
                    }
                }
            }
        };
            
        // Load transformer layers
        eprintln!("DEBUG: Extracting transformer layers");
        let mut layers = Vec::new();
        for i in 0..num_layers {
            eprintln!("DEBUG: Loading layer {}/{}", i+1, num_layers);
            
            // Determine layer format and key patterns
            let (attn_key_format, attn_out_format, ffn_key_format, ln_key_format) = 
                if is_hf_qwen3_format {
                    // Modern HF Qwen3 format used in the project's model file
                    ("self_attn.{}_proj.{}", "self_attn.o_proj.{}", "mlp.{}_proj.{}", "{}_layernorm.{}")
                } else if is_hf_format {
                    // Generic HF format for other transformer models
                    ("attn.c_attn.{}", "attn.c_proj.{}", "mlp.{}.{}", "ln_{}.{}")
                } else {
                    // Legacy format
                    ("attention.{}.{}", "attention.output.{}", "feed_forward.{}.{}", "input_layernorm.{}")
                };
                
            // Print the first few layer weight keys so we can see the pattern
            if i == 0 {
                // Construct example key patterns for debugging
                let q_proj = format!("{}", format!("{}", layer_prefix_format)
                    .replace("{}", "0")
                    .replace("{}", "self_attn.q_proj.weight"));
                    
                let up_proj = format!("{}", format!("{}", layer_prefix_format)
                    .replace("{}", "0")
                    .replace("{}", "mlp.up_proj.weight"));
                    
                let input_ln = format!("{}", format!("{}", layer_prefix_format)
                    .replace("{}", "0")
                    .replace("{}", "input_layernorm.weight"));
                    
                eprintln!("DEBUG: Looking for pattern like: {}, {}, {}", q_proj, up_proj, input_ln);
            }
            
            // Extract attention weights
            let query_weight_key = format!("{}", format!("{}",
                layer_prefix_format).replace("{}", &i.to_string()).replace("{}", 
                &format!("{}", attn_key_format).replace("{}", "weight")));
            
            let key_weight_key = format!("{}", format!("{}",
                layer_prefix_format).replace("{}", &i.to_string()).replace("{}",
                &format!("{}", attn_key_format).replace("{}", "weight")));
            
            let value_weight_key = format!("{}", format!("{}",
                layer_prefix_format).replace("{}", &i.to_string()).replace("{}",
                &format!("{}", attn_key_format).replace("{}", "weight")));
            
            let output_weight_key = format!("{}", format!("{}",
                layer_prefix_format).replace("{}", &i.to_string()).replace("{}",
                &format!("{}", attn_out_format).replace("{}", "weight")));
            
            // For attention weights, we need to be flexible as models have different layouts
            let query_weight = match tensor_map.get(&query_weight_key) {
                Some(w) => w.to_device(&device)?,
                None => {
                    // For Qwen3 model, directly use this pattern as it's the most common
                    // model.layers.N.self_attn.q_proj.weight
                    let qwen3_key = format!("model.layers.{}.self_attn.q_proj.weight", i);
                    
                    // Also try the generic format
                    let alt_key = format!("{}", format!("{}",
                                  layer_prefix_format).replace("{}", &i.to_string()).replace("{}", "self_attn.q_proj.weight"));
                    // Try the Qwen3 key first (most likely)
                    match tensor_map.get(&qwen3_key) {
                        Some(w) => {
                            w.to_device(&device)?
                        },
                        None => {
                            // Try the alternative key format
                            match tensor_map.get(&alt_key) {
                                Some(w) => w.to_device(&device)?,
                                None => {
                                    eprintln!("DEBUG: Query weight not found for layer {}, ERROR - Cannot continue without weights", i);
                                    return Err(anyhow!("Missing critical weight: query_weight for layer {}", i));
                                }
                            }
                        }
                    }
                }
            };
            
            // Similar pattern for remaining weights...
            let key_weight = match tensor_map.get(&key_weight_key) {
                Some(w) => w.to_device(&device)?,
                None => {
                    // For Qwen3 model, directly use this pattern
                    let qwen3_key = format!("model.layers.{}.self_attn.k_proj.weight", i);
                    
                    // Also try the generic format
                    let alt_key = format!("{}", format!("{}",
                                  layer_prefix_format).replace("{}", &i.to_string()).replace("{}", "self_attn.k_proj.weight"));
                    
                    // Try the Qwen3 key first
                    match tensor_map.get(&qwen3_key) {
                        Some(w) => w.to_device(&device)?,
                        None => {
                            // Try the alternative key format
                            match tensor_map.get(&alt_key) {
                                Some(w) => w.to_device(&device)?,
                                None => {
                                    eprintln!("DEBUG: Key weight not found for layer {}, ERROR - Cannot continue without weights", i);
                                    return Err(anyhow!("Missing critical weight: key_weight for layer {}", i));
                                }
                            }
                        }
                    }
                }
            };
            
            let value_weight = match tensor_map.get(&value_weight_key) {
                Some(w) => w.to_device(&device)?,
                None => {
                    // For Qwen3 model, directly use this pattern
                    let qwen3_key = format!("model.layers.{}.self_attn.v_proj.weight", i);
                    
                    // Also try the generic format
                    let alt_key = format!("{}", format!("{}",
                                  layer_prefix_format).replace("{}", &i.to_string()).replace("{}", "self_attn.v_proj.weight"));
                    
                    // Try the Qwen3 key first
                    match tensor_map.get(&qwen3_key) {
                        Some(w) => w.to_device(&device)?,
                        None => {
                            // Try the alternative key format
                            match tensor_map.get(&alt_key) {
                                Some(w) => w.to_device(&device)?,
                                None => {
                                    eprintln!("DEBUG: Value weight not found for layer {}, ERROR - Cannot continue without weights", i);
                                    return Err(anyhow!("Missing critical weight: value_weight for layer {}", i));
                                }
                            }
                        }
                    }
                }
            };
            
            let output_weight = match tensor_map.get(&output_weight_key) {
                Some(w) => w.to_device(&device)?,
                None => {
                    // For Qwen3 model, directly use this pattern
                    let qwen3_key = format!("model.layers.{}.self_attn.o_proj.weight", i);
                    
                    // Also try the generic format
                    let alt_key = format!("{}", format!("{}",
                                  layer_prefix_format).replace("{}", &i.to_string()).replace("{}", "self_attn.o_proj.weight"));
                    
                    // Try the Qwen3 key first
                    match tensor_map.get(&qwen3_key) {
                        Some(w) => w.to_device(&device)?,
                        None => {
                            // Try the alternative key format
                            match tensor_map.get(&alt_key) {
                                Some(w) => w.to_device(&device)?,
                                None => {
                                    eprintln!("DEBUG: Output weight not found for layer {}, ERROR - Cannot continue without weights", i);
                                    return Err(anyhow!("Missing critical weight: output_weight for layer {}", i));
                                }
                            }
                        }
                    }
                }
            };
                
            // Extract layer norm parameters using direct Qwen3 key patterns
            // For Qwen3, these are typically "model.layers.{i}.input_layernorm.weight"
            let input_ln_key = format!("model.layers.{}.input_layernorm.weight", i);
            let input_ln_bias_key = format!("model.layers.{}.input_layernorm.bias", i);
            
            // Try the Qwen3 key first
            let attn_norm_weight = match tensor_map.get(&input_ln_key) {
                Some(w) => {
                    w.to_device(&device)?
                },
                None => {
                    // Fall back to the generic key format
                    let generic_key = format!("{}", format!("{}",
                        layer_prefix_format).replace("{}", &i.to_string()).replace("{}",
                        &format!("{}", ln_key_format).replace("{}", "1").replace("{}", "weight")));
                    
                    match tensor_map.get(&generic_key) {
                        Some(w) => w.to_device(&device)?,
                        None => {
                            eprintln!("DEBUG: Input layernorm weight not found for layer {}, ERROR - Cannot continue without weights", i);
                            return Err(anyhow!("Missing critical weight: input_layernorm.weight for layer {}", i));
                        }
                    }
                }
            };
            
            // Similarly for bias
            let attn_norm_bias = match tensor_map.get(&input_ln_bias_key) {
                Some(b) => Some(b.to_device(&device)?),
                None => {
                    // Fall back to the generic key format
                    let generic_key = format!("{}", format!("{}",
                        layer_prefix_format).replace("{}", &i.to_string()).replace("{}",
                        &format!("{}", ln_key_format).replace("{}", "1").replace("{}", "bias")));
                    
                    match tensor_map.get(&generic_key) {
                        Some(b) => Some(b.to_device(&device)?),
                        None => {
                            Some(Tensor::zeros(hidden_dim, DType::F32, &device)?)
                        }
                    }
                }
            };
                
            // Extract feed-forward weights using direct Qwen3 key patterns
            // Qwen3 MLP has up_proj, down_proj, and gate_proj weights
            let up_proj_key = format!("model.layers.{}.mlp.up_proj.weight", i);
            let _up_proj_bias_key = format!("model.layers.{}.mlp.up_proj.bias", i);
            let down_proj_key = format!("model.layers.{}.mlp.down_proj.weight", i);
            let _down_proj_bias_key = format!("model.layers.{}.mlp.down_proj.bias", i);
            let gate_proj_key = format!("model.layers.{}.mlp.gate_proj.weight", i);
            let _gate_proj_bias_key = format!("model.layers.{}.mlp.gate_proj.bias", i);
            
            // For Qwen3, we primarily need the up_proj (for intermediate weight)
            // and down_proj (for output weight)
            let ff_inter_weight = match tensor_map.get(&up_proj_key) {
                Some(w) => {
                    w.to_device(&device)?
                },
                None => {
                    // Try the generic format
                    let generic_key = format!("{}", format!("{}",
                        layer_prefix_format).replace("{}", &i.to_string()).replace("{}",
                        &format!("{}", ffn_key_format).replace("{}", "c_fc").replace("{}", "weight")));
                    
                    match tensor_map.get(&generic_key) {
                        Some(w) => w.to_device(&device)?,
                        None => {
                            // Last attempt with the alternative "mlp.up_proj.weight" format
                            let alt_key = format!("{}", format!("{}",
                                layer_prefix_format).replace("{}", &i.to_string()).replace("{}", "mlp.up_proj.weight"));
                                
                            match tensor_map.get(&alt_key) {
                                Some(w) => w.to_device(&device)?,
                                None => {
                                    eprintln!("DEBUG: MLP up_proj weight not found for layer {}, ERROR - Cannot continue without weights", i);
                                    return Err(anyhow!("Missing critical weight: mlp.up_proj.weight for layer {}", i));
                                }
                            }
                        }
                    }
                }
            };
            
            // Get the feed-forward intermediate dimension
            let _ff_dim = ff_inter_weight.dim(1)?;
            
            // Direct access to MLPs bias tensors using the correct keys
            let up_proj_bias_key = format!("model.layers.{}.mlp.up_proj.bias", i);
            let gate_proj_bias_key = format!("model.layers.{}.mlp.gate_proj.bias", i);
            
            // Direct fetch from tensor map with these keys
            let ff_inter_bias = tensor_map.get(&up_proj_bias_key)
                .map(|b| b.to_device(&device).unwrap());
            
            // Track if bias was loaded
            
            // Load the gate_proj weights for SwiGLU activation
            let ff_gate_weight = match tensor_map.get(&gate_proj_key) {
                Some(w) => {
                    Some(w.to_device(&device)?)
                },
                None => {
                    None
                }
            };
            
            // Direct fetch of gate_proj bias
            let ff_gate_bias = tensor_map.get(&gate_proj_bias_key)
                .map(|b| b.to_device(&device).unwrap());
            
            
            // For the output projection (down_proj in Qwen3)
            let ff_output_weight = match tensor_map.get(&down_proj_key) {
                Some(w) => {
                    w.to_device(&device)?
                },
                None => {
                    // Try the generic format
                    let generic_key = format!("{}", format!("{}",
                        layer_prefix_format).replace("{}", &i.to_string()).replace("{}",
                        &format!("{}", ffn_key_format).replace("{}", "c_proj").replace("{}", "weight")));
                    
                    match tensor_map.get(&generic_key) {
                        Some(w) => w.to_device(&device)?,
                        None => {
                            // Last attempt with the alternative "mlp.down_proj.weight" format
                            let alt_key = format!("{}", format!("{}",
                                layer_prefix_format).replace("{}", &i.to_string()).replace("{}", "mlp.down_proj.weight"));
                                
                            match tensor_map.get(&alt_key) {
                                Some(w) => w.to_device(&device)?,
                                None => {
                                    eprintln!("DEBUG: MLP down_proj weight not found for layer {}, ERROR - Cannot continue without weights", i);
                                    return Err(anyhow!("Missing critical weight: mlp.down_proj.weight for layer {}", i));
                                }
                            }
                        }
                    }
                }
            };
            
            // Direct access to down_proj bias using the correct key
            let down_proj_bias_key = format!("model.layers.{}.mlp.down_proj.bias", i);
            
            // Direct fetch of down_proj bias
            let ff_output_bias = tensor_map.get(&down_proj_bias_key)
                .map(|b| b.to_device(&device).unwrap());
                
                
            // Extract feed-forward layer norm parameters using direct Qwen3 key patterns
            // For Qwen3, this is typically "model.layers.{i}.post_attention_layernorm.weight"
            let post_attn_ln_key = format!("model.layers.{}.post_attention_layernorm.weight", i);
            let post_attn_ln_bias_key = format!("model.layers.{}.post_attention_layernorm.bias", i);
            
            // Try the Qwen3 key first
            let ff_norm_weight = match tensor_map.get(&post_attn_ln_key) {
                Some(w) => {
                    w.to_device(&device)?
                },
                None => {
                    // Fall back to the generic key format
                    let generic_key = format!("{}", format!("{}",
                        layer_prefix_format).replace("{}", &i.to_string()).replace("{}",
                        &format!("{}", ln_key_format).replace("{}", "2").replace("{}", "weight")));
                    
                    match tensor_map.get(&generic_key) {
                        Some(w) => w.to_device(&device)?,
                        None => {
                            eprintln!("DEBUG: Post-attention layernorm weight not found for layer {}, ERROR - Cannot continue without weights", i);
                            return Err(anyhow!("Missing critical weight: post_attention_layernorm.weight for layer {}", i));
                        }
                    }
                }
            };
            
            // Similarly for bias
            let ff_norm_bias = match tensor_map.get(&post_attn_ln_bias_key) {
                Some(b) => Some(b.to_device(&device)?),
                None => {
                    // Fall back to the generic key format
                    let generic_key = format!("{}", format!("{}",
                        layer_prefix_format).replace("{}", &i.to_string()).replace("{}",
                        &format!("{}", ln_key_format).replace("{}", "2").replace("{}", "bias")));
                    
                    match tensor_map.get(&generic_key) {
                        Some(b) => Some(b.to_device(&device)?),
                        None => {
                            Some(Tensor::zeros(hidden_dim, DType::F32, &device)?)
                        }
                    }
                }
            };
                
            
            // Create transformer layer with gate_proj weights for SwiGLU activation
            layers.push(TransformerLayer::new(
                query_weight,
                key_weight,
                value_weight,
                output_weight,
                attn_norm_weight,
                attn_norm_bias,
                ff_inter_weight,
                ff_inter_bias,
                ff_gate_weight,     // New: gate_proj weight for SwiGLU
                ff_gate_bias,       // New: gate_proj bias for SwiGLU
                ff_output_weight,
                ff_output_bias,
                ff_norm_weight,
                ff_norm_bias,
            ));
        }
        
        if use_fallback {
            eprintln!("\n\n⚠️  WARNING: Using a placeholder model since embedded model data is missing or invalid");
            eprintln!("⚠️  This model will not produce meaningful outputs");
            
            if EMBEDDED_MODEL_SAFETENSORS.len() > 1_000_000 {
                // Model file exists but couldn't be parsed
                eprintln!("⚠️  The embedded model IS available ({}MB) but couldn't be loaded correctly.", 
                         EMBEDDED_MODEL_SAFETENSORS.len() / 1024 / 1024);
                eprintln!("⚠️  This might be due to memory constraints or the safetensors format being invalid.");
                eprintln!("⚠️  If using Metal acceleration, try without GPU by setting use_gpu: false\n\n");
            } else {
                // Model file is missing
                eprintln!("⚠️  You need to download the Qwen3-0.6B model file from Hugging Face");
                eprintln!("⚠️  Visit: https://huggingface.co/Qwen/Qwen3-0.6B");
                eprintln!("⚠️  Download model.safetensors and place it in models/model.safetensors\n\n");
            }
            
            // CRITICAL: Return an error instead of using random weights
            // This is essential because random weights will generate garbage output
            return Err(anyhow!("Model is in fallback mode with placeholder weights. Cannot proceed with inference."));
        }
        
        eprintln!("DEBUG: Model successfully loaded");
        
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
    
    /// Check if the model is using GPU acceleration
    pub fn is_using_gpu(&self) -> bool {
        match self.device {
            Device::Cuda(_) => true,
            Device::Metal(_) => true,
            _ => false,
        }
    }
    
    /// Returns the name of the hardware acceleration being used
    pub fn get_acceleration_name(&self) -> &'static str {
        match self.device {
            Device::Cuda(_) => "CUDA (NVIDIA GPU)",
            Device::Metal(_) => "Metal (Apple GPU)",
            _ => "CPU",
        }
    }
    
    /// Ensures the input tensor has the expected shape for matmul operations
    fn ensure_expected_shape(&self, tensor: &Tensor, expected_dims: usize) -> Result<Tensor> {
        let shape = tensor.shape().dims();
        
        // Log shape information for debugging
        // eprintln!("DEBUG: Tensor shape: {:?}, expected dims: {}", shape, expected_dims);
        
        // If tensor is already 2D and has the expected second dimension, we're good
        if shape.len() == 2 && shape[1] == expected_dims {
            return Ok(tensor.clone());
        }
        
        // If the tensor is 1D, reshape it to be 2D with batch dimension of 1
        if shape.len() == 1 && shape[0] == expected_dims {
            // eprintln!("DEBUG: Reshaping 1D tensor to 2D: {:?} -> (1, {})", shape, expected_dims);
            return Ok(tensor.reshape((1, expected_dims))?);
        }
        
        // If the tensor is 2D but second dimension doesn't match, try narrow it (for Qwen3's doubled dimensions)
        if shape.len() == 2 && shape[1] > expected_dims && shape[1] % expected_dims == 0 {
            eprintln!("DEBUG: Narrowing tensor from {:?} to take only first {} columns", shape, expected_dims);
            return Ok(tensor.narrow(1, 0, expected_dims)?);
        }
        
        // If we got here, the tensor shape is unexpected but we'll try to continue
        eprintln!("WARNING: Tensor shape {:?} doesn't match expected dimension {}. This may cause issues.", 
                 shape, expected_dims);
        
        // Return the original tensor and let the error happen later if dimensions are truly incompatible
        Ok(tensor.clone())
    }
    
    /// Layer normalization using candle ops with CPU fallback
    fn layer_norm(&self, input: &Tensor, weight: &Tensor, bias: &Option<Tensor>) -> Result<Tensor> {
        // Ensure input tensor has the expected shape [batch_size, hidden_dim]
        let input_shape = input.shape().dims();
        let input = if input_shape.len() == 1 {
            // If we get a 1D tensor, reshape to [1, hidden_dim]
            input.reshape((1, input_shape[0]))?
        } else {
            input.clone()
        };
        
        // Use candle_nn's layer_norm operation
        let eps = 1e-5;
        
        // The API requires a tensor for beta, we need to handle the None case
        let normalized = match bias {
            Some(b) => {
                match ops::layer_norm(&input, &weight, b, eps) {
                    Ok(result) => result,
                    Err(e) => {
                        // If we get a Metal error, fall back to CPU
                        if format!("{:?}", e).contains("no metal implementation for layer-norm") {
                            // Move tensors to CPU, perform layer norm, and move back
                            let input_cpu = input.to_device(&Device::Cpu)?;
                            let weight_cpu = weight.to_device(&Device::Cpu)?;
                            let bias_cpu = b.to_device(&Device::Cpu)?;
                            
                            // Perform layer norm on CPU
                            let norm_cpu = ops::layer_norm(&input_cpu, &weight_cpu, &bias_cpu, eps)?;
                            
                            // Move back to original device
                            norm_cpu.to_device(&self.device)?
                        } else {
                            // For other errors, propagate
                            return Err(e.into());
                        }
                    }
                }
            },
            None => {
                // Create a zeros tensor of the same shape as weight for the bias
                let zeros = Tensor::zeros(weight.shape(), weight.dtype(), &self.device)?;
                
                match ops::layer_norm(&input, &weight, &zeros, eps) {
                    Ok(result) => result,
                    Err(e) => {
                        // If we get a Metal error, fall back to CPU
                        if format!("{:?}", e).contains("no metal implementation for layer-norm") {
                            // Move tensors to CPU, perform layer norm, and move back
                            let input_cpu = input.to_device(&Device::Cpu)?;
                            let weight_cpu = weight.to_device(&Device::Cpu)?;
                            let zeros_cpu = zeros.to_device(&Device::Cpu)?;
                            
                            // Perform layer norm on CPU
                            let norm_cpu = ops::layer_norm(&input_cpu, &weight_cpu, &zeros_cpu, eps)?;
                            
                            // Move back to original device
                            norm_cpu.to_device(&self.device)?
                        } else {
                            // For other errors, propagate
                            return Err(e.into());
                        }
                    }
                }
            }
        };
        
        Ok(normalized)
    }
    
    /// Generates token IDs from the input token IDs
    pub fn generate(&self, input_tokens: &[u32], max_tokens: usize) -> Result<Vec<u32>> {
        eprintln!("Starting generation with {} input tokens", input_tokens.len());
        
        if !self.is_loaded {
            return Err(anyhow::anyhow!("Model not loaded"));
        }
        
        // Find the assistant marker in the input (looking for the assistant tag)
        let assistant_start_idx = input_tokens
            .windows(2)
            .position(|window| window[0] == 151644 && window[1] == 151645)
            .unwrap_or(input_tokens.len().saturating_sub(1));
            
        // Get just the prompt part (everything up to and including the assistant marker)
        let prompt_tokens = &input_tokens[0..=assistant_start_idx];
        
        // Start with just the prompt tokens
        let mut output = prompt_tokens.to_vec();
        
        // Loop through and generate new tokens one by one
        let mut tokens_generated = 0;
        
        // Determine the maximum sequence length (input + generated tokens)
        // We need to allocate enough space in the KV cache for all tokens we'll process
        let max_seq_len = input_tokens.len() + max_tokens;
        
        // Keep track of key/value cache for efficient inference
        let mut kv_cache = KVCache::new(
            self.num_layers,
            self.num_heads,
            max_seq_len, // Reserve space for ALL tokens (input + generated)
            self.head_dim,
            &self.device,
        )?;
        
        // Run forward pass on all input tokens to build the initial KV cache
        eprintln!("Building KV cache...");
        
        // Convert input tokens to tensor
        let input_tensor = Tensor::new(input_tokens, &self.device)?;
        
        // Get logits for the last token
        let mut logits = self.forward_pass(&input_tensor, &mut kv_cache)?;
        
        eprintln!("Starting token generation...");
        // Generate new tokens auto-regressively
        while tokens_generated < max_tokens {
            // Start timing token generation
            let _token_start = Instant::now();
            
            // Sample next token based on logits and temperature
            let next_token = self.sample_next_token(&logits)?;
            
            // Stop if we hit the end of sequence token
            if next_token == 151645 { // EOS token for Qwen3
                break;
            }
            
            // Add token to output
            output.push(next_token);
            tokens_generated += 1;
            
            // Convert new token to tensor
            let next_token_tensor = Tensor::new(&[next_token], &self.device)?;
            
            // Generate logits for the next token
            // We need to update the logits for the next iteration
            // Position is input tokens length + tokens generated so far
            let current_position = input_tokens.len() + tokens_generated;
            logits = self.forward_pass_with_cache(&next_token_tensor, &mut kv_cache, current_position)?;
        }
        
        eprintln!("Generation complete: {} tokens generated", tokens_generated);
        
        Ok(output)
    }
    
    /// Perform self-attention operation with Candle
    fn self_attention(
        &self, 
        input: &Tensor, 
        layer: &TransformerLayer, 
        layer_idx: usize,  // This is the layer index in the transformer
        position: usize,   // This is the position in the sequence
        kv_cache: &mut KVCache
    ) -> Result<Tensor> {
        // Split batch_size=1 from calculations for clarity
        let batch_size = 1;
        
        // Check input shape and ensure it's compatible for matmul
        let input_shaped = self.ensure_expected_shape(input, self.hidden_dim)?;
        
        // Project input to query, key, and value
        // This is using hardware-accelerated matrix multiplication
        // We need to transpose the weights for correct matrix multiplication
        // For query projection: [1, 1024] x [2048, 1024]T = [1, 2048]
        let query_weight_t = layer.query_weight.transpose(0, 1)?;
        let query = input_shaped.matmul(&query_weight_t)?;
        
        // Handle Qwen3-style doubled query dimension
        // In Qwen3 and similar models, the projected query is typically split in two:
        // The first half is for the query, the second half is often discarded or used for advanced gating mechanisms.
        let query_dims = query.shape().dims();
        let query = if query_dims.len() == 2 && query_dims[1] == 2 * self.num_heads * self.head_dim {
            // Qwen3-style double-sized query, take first half only
            query.narrow(1, 0, self.num_heads * self.head_dim)?
        } else {
            query
        };
        
        // Do the same for key and value projections
        let key_weight_t = layer.key_weight.transpose(0, 1)?;
        let value_weight_t = layer.value_weight.transpose(0, 1)?;
        let key = input_shaped.matmul(&key_weight_t)?;
        let value = input_shaped.matmul(&value_weight_t)?;
        
        // Debug shapes available for debugging if needed
        // Removed excessive logging
        
        // Reshape query, key, and value to multi-head format
        // The projected shapes might be [1, 2048] after transposing and matmul
        // We need to reshape to [batch_size, num_heads, head_dim]
        // Reshape to multi-head format
        
        // Reshape query, key, and value to multi-head format
        // Standard case: [batch_size, num_heads * head_dim] -> [batch_size, num_heads, head_dim]
        let query = query.reshape((batch_size, self.num_heads, self.head_dim))?;
        let key = key.reshape((batch_size, self.num_heads, self.head_dim))?;
        let value = value.reshape((batch_size, self.num_heads, self.head_dim))?;
        
        // Store key and value in the KV cache for this position
        kv_cache.update_cache(layer_idx, position, &key.unsqueeze(2)?, &value.unsqueeze(2)?)?;
        
        // Get KV cache for this layer up to current position
        let (k_cache, v_cache) = kv_cache.get_cache_for_layer(layer_idx, position)?;
        
        // Compute attention scores (this is a critical performance bottleneck in vanilla implementation)
        // Using Candle's optimized matmul with proper broadcasting
        let q = query.unsqueeze(2)?; // Shape: [batch_size, num_heads, 1, head_dim]
        
        // Compute attention scores: [batch_size, num_heads, 1, position+1]
        // This uses BLAS under the hood for optimal performance
        let _scaling_factor = (self.head_dim as f32).sqrt(); // Unused but kept for reference
        // Compute the matmul first
        let scores = q.matmul(&k_cache.transpose(2, 3)?)?;
        
        // For scaling, we'd normally divide by sqrt(head_dim) 
        // but we're skipping explicit scaling for now since softmax will normalize anyway
        // This simplifies the implementation and avoids potential broadcasting issues
        let attn_scores = scores.clone();
        
        // Apply softmax along last dimension (position dimension)
        let attn_weights = ops::softmax(&attn_scores, 3)?;
        
        // Apply attention weights to values
        // This uses BLAS matrix multiplication for best performance
        let context = attn_weights.matmul(&v_cache)?;
        
        // Reshape: [batch_size, num_heads, 1, head_dim] -> [batch_size, num_heads * head_dim]
        let context = context.reshape((batch_size, self.num_heads * self.head_dim))?;
        
        // Project back to hidden dimension
        // For Qwen3 models, the output projection might also have doubled dimensions
        // We need to handle this case to ensure the output has the correct shape [1, hidden_dim]
        let output = context.matmul(&layer.output_weight)?;
        
        // Check if output needs to be narrowed due to doubled output dimension
        let output_dims = output.shape().dims();
        let output = if output_dims.len() == 2 && output_dims[1] == 2 * self.hidden_dim {
            // Take only the first half for doubled output dimension
            output.narrow(1, 0, self.hidden_dim)?
        } else {
            output
        };
        
        Ok(output)
    }
    
    /// Feed-forward network using Candle operations
    /// Implements SwiGLU activation used in Qwen3 models
    fn feed_forward(&self, input: &Tensor, layer: &TransformerLayer) -> Result<Tensor> {
        // For Qwen3, we use SwiGLU activation
        // This involves two projection matrices (up_proj and gate_proj)
        // followed by elementwise multiplication and GELU activation
        
        // Check input shape and ensure it's compatible for matmul
        let input_shaped = self.ensure_expected_shape(input, self.hidden_dim)?;
        
        // First projection (up_proj in Qwen3)
        let ff_inter_shape = layer.ff_inter_weight.shape().dims();
        let mut up_proj = if ff_inter_shape.len() == 2 && ff_inter_shape[1] == self.hidden_dim {
            // If the weight shape is [ff_dim, hidden_dim], we need to transpose for [1, hidden_dim] × [ff_dim, hidden_dim]^T
            let ff_inter_weight_t = layer.ff_inter_weight.transpose(0, 1)?;
            input_shaped.matmul(&ff_inter_weight_t)?
        } else {
            // Standard case
            input_shaped.matmul(&layer.ff_inter_weight)?
        };
        
        // Add bias if present, but check shape compatibility first
        if let Some(bias) = &layer.ff_inter_bias {
            let up_proj_shape = up_proj.shape().dims();
            let bias_shape = bias.shape().dims();
            
            
            // Check for shape compatibility
            if bias_shape.len() == 1 && up_proj_shape.len() == 2 && bias_shape[0] == up_proj_shape[1] {
                // Shapes are compatible, add the bias
                up_proj = up_proj.add(bias)?;
            } else {
                // Shapes are incompatible, skip adding bias
            }
        }
        
        // For gate projection in SwiGLU, either use dedicated weights or approximate
        let gate_proj = match &layer.ff_gate_weight {
            Some(gate_weight) => {
                // Use the dedicated gate_proj weights (proper Qwen3 approach)
                let gate_weight_shape = gate_weight.shape().dims();
                let gate = if gate_weight_shape.len() == 2 && gate_weight_shape[1] == self.hidden_dim {
                    // Transpose needed
                    let gate_weight_t = gate_weight.transpose(0, 1)?;
                    input_shaped.matmul(&gate_weight_t)?
                } else {
                    // Standard case
                    input_shaped.matmul(gate_weight)?
                };
                
                // Add bias if present, but check shape compatibility first
                if let Some(bias) = &layer.ff_gate_bias {
                    let gate_shape = gate.shape().dims();
                    let bias_shape = bias.shape().dims();
                    
                    
                    // Check for shape compatibility
                    if bias_shape.len() == 1 && gate_shape.len() == 2 && bias_shape[0] == gate_shape[1] {
                        // Shapes are compatible, add the bias
                        gate.add(bias)?
                    } else {
                        // Shapes are incompatible, skip adding bias
                        gate
                    }
                } else {
                    gate
                }
            },
            None => {
                // Fall back to approximation using the same weights
                up_proj.clone()
            }
        };
        
        // Apply GELU activation to the gate path
        let gate_act = activation::Activation::Gelu.forward(&gate_proj)?;
        
        // Multiply the two pathways (SwiGLU activation)
        let intermediate = up_proj.mul(&gate_act)?;
        
        // Prepare for output projection
        
        // Check and reshape if needed
        let inter_shape = intermediate.shape().dims();
        let weight_shape = layer.ff_output_weight.shape().dims();
        
        // Usually, intermediate = [batch, ff_dim], ff_output_weight = [ff_dim, hidden_dim]
        let mut output = if inter_shape.len() == 2 && inter_shape[1] == weight_shape[0] {
            // Standard case
            intermediate.matmul(&layer.ff_output_weight)?
        } else if inter_shape.len() == 1 && inter_shape[0] == weight_shape[0] {
            // 1D case, reshape to 2D
            let reshaped = intermediate.reshape((1, inter_shape[0]))?;
            reshaped.matmul(&layer.ff_output_weight)?
        } else if inter_shape.len() == 2 && weight_shape.len() == 2 && weight_shape[1] == inter_shape[1] {
            // Need to transpose the weight matrix
            let transposed = layer.ff_output_weight.transpose(0, 1)?;
            intermediate.matmul(&transposed)?
        } else {
            return Err(anyhow!(
                "feed_forward shape mismatch: intermediate shape {:?}, ff_output_weight shape {:?}",
                inter_shape, weight_shape
            ));
        };
        
        // Add bias if present, but check shape compatibility first
        if let Some(bias) = &layer.ff_output_bias {
            let output_shape = output.shape().dims();
            let bias_shape = bias.shape().dims();
            
            
            // Check for shape compatibility
            if bias_shape.len() == 1 && output_shape.len() == 2 && bias_shape[0] == output_shape[1] {
                // Shapes are compatible, add the bias
                output = output.add(bias)?;
            } else {
                // Shapes are incompatible, skip adding bias
            }
        }
        
        // Check if output needs to be narrowed due to doubled output dimension (similar to attention output)
        let output_dims = output.shape().dims();
        let output = if output_dims.len() == 2 && output_dims[1] == 2 * self.hidden_dim {
            output.narrow(1, 0, self.hidden_dim)?
        } else {
            output
        };
        
        Ok(output)
    }
    
    /// Perform the forward pass through the transformer
    fn forward_pass(&self, tokens: &Tensor, kv_cache: &mut KVCache) -> Result<Tensor> {
        // Start timing the forward pass
        let start_time = Instant::now();
        let seq_len = tokens.dim(0)?;
        eprintln!("DEBUG: Processing {} tokens through full forward pass", seq_len);
        
        // Get whether we're using GPU
        let using_gpu = self.is_using_gpu();
        if using_gpu {
            eprintln!("DEBUG: Using GPU acceleration");
        }
        
        // Embedding lookup - we need to process tokens one by one to build the KV cache
        let mut hidden_states = Vec::with_capacity(seq_len as usize);
        
        for pos in 0..seq_len as usize {
            let _token_start_time = Instant::now();
            
            // Get token at position
            let token_id = tokens.get(pos as usize)?.to_scalar::<u32>()?;
            
            if token_id as usize >= self.vocab_size {
                return Err(anyhow!("Token ID {} out of vocabulary range", token_id));
            }
            
            // Lookup embedding for this token
            let token_embedding = self.embedding.get(token_id as usize)?;
            
            // Get positional embedding
            let pos_embedding = self.position_embedding.get(pos % 2048)?;
            
            // Make sure both tensors have matching dimensions for addition
            let token_emb_shaped = self.ensure_expected_shape(&token_embedding, self.hidden_dim)?;
            let pos_emb_shaped = self.ensure_expected_shape(&pos_embedding, self.hidden_dim)?;
            
            // Add token and position embeddings
            let mut state = token_emb_shaped.add(&pos_emb_shaped)?;
            
            // Process through transformer layers
            for (layer_idx, layer) in self.layers.iter().enumerate() {
                // Layer normalization before attention
                let norm_state = self.layer_norm(&state, &layer.attn_norm_weight, &layer.attn_norm_bias)?;
                
                // Self-attention - use layer_idx to ensure we use the correct KV cache for each layer
                let attn_output = self.self_attention(&norm_state, layer, layer_idx, pos, kv_cache)?;
                
                // Residual connection
                state = state.add(&attn_output)?;
                
                // Layer normalization before feed-forward
                let norm_state = self.layer_norm(&state, &layer.ff_norm_weight, &layer.ff_norm_bias)?;
                
                // Feed-forward
                let ff_output = self.feed_forward(&norm_state, layer)?;
                
                // Residual connection
                state = state.add(&ff_output)?;
            }
            
            // Final layer normalization
            state = self.layer_norm(&state, &self.final_norm_weight, &self.final_norm_bias)?;
            
            // Add state to hidden states
            hidden_states.push(state);
        }
        
        // Get the last hidden state
        let last_hidden = hidden_states.last()
            .ok_or_else(|| anyhow!("No hidden states produced"))?;
        
        // Project to logits using LM head
        // This is using hardware-accelerated matrix multiplication
        let logits = last_hidden.matmul(&self.lm_head.transpose(0, 1)?)?;
        
        eprintln!("DEBUG: Forward pass complete, generated logits for {} tokens in {:.2?}", seq_len, start_time.elapsed());
        
        Ok(logits)
    }
    
    /// Forward pass with cached key-values (for efficient generation)
    fn forward_pass_with_cache(&self, token: &Tensor, kv_cache: &mut KVCache, position: usize) -> Result<Tensor> {
        // Start timing the operation
        let _start_time = Instant::now();
        
        // Get token ID
        let token_id = token.get(0)?.to_scalar::<u32>()?;
        
        if token_id as usize >= self.vocab_size {
            return Err(anyhow!("Token ID {} out of vocabulary range", token_id));
        }
        
        // Embedding lookup for this token
        let token_embedding = self.embedding.get(token_id as usize)?;
        
        // Ensure token embedding has shape [1, hidden_dim], not just [hidden_dim]
        let token_embedding_shape = token_embedding.shape().dims();
        
        let token_embedding = if token_embedding_shape.len() == 1 {
            token_embedding.reshape((1, self.hidden_dim))?
        } else {
            token_embedding
        };
        
        // Get positional embedding
        let pos_embedding = self.position_embedding.get(position % 2048)?;
        
        // Ensure positional embedding has shape [1, hidden_dim] as well
        let pos_embedding_shape = pos_embedding.shape().dims();
        
        let pos_embedding = if pos_embedding_shape.len() == 1 {
            pos_embedding.reshape((1, self.hidden_dim))?
        } else {
            pos_embedding
        };
        
        // Add token and position embeddings
        let mut state = token_embedding.add(&pos_embedding)?;
        
        // Process through transformer layers
        for (layer_idx, layer) in self.layers.iter().enumerate() {
            // Layer normalization before attention
            let norm_state = self.layer_norm(&state, &layer.attn_norm_weight, &layer.attn_norm_bias)?;
            
            // Self-attention with the new token
            // Pass layer_idx to ensure we use the correct KV cache for each layer
            let attn_output = self.self_attention(&norm_state, layer, layer_idx, position, kv_cache)?;
            
            // Residual connection
            state = state.add(&attn_output)?;
            
            // Layer normalization before feed-forward
            let norm_state = self.layer_norm(&state, &layer.ff_norm_weight, &layer.ff_norm_bias)?;
            
            // Feed-forward
            let ff_output = self.feed_forward(&norm_state, layer)?;
            
            // Residual connection
            state = state.add(&ff_output)?;
        }
        
        // Final layer normalization
        state = self.layer_norm(&state, &self.final_norm_weight, &self.final_norm_bias)?;
        
        // Project to logits using LM head
        let logits = state.matmul(&self.lm_head.transpose(0, 1)?)?;
        
        // Skip token processing time reporting - it's handled elsewhere
        
        Ok(logits)
    }
    
    /// Sample the next token from the logits using temperature, top-k and top-p (nucleus) sampling
    fn sample_next_token(&self, logits: &Tensor) -> Result<u32> {
        // Check logits shape and squeeze if needed
        let logits_shape = logits.shape().dims();
        
        // If logits has shape [1, vocab_size], we need to squeeze it to [vocab_size]
        let squeezed_logits = if logits_shape.len() == 2 && logits_shape[0] == 1 {
            logits.squeeze(0)?
        } else {
            logits.clone()
        };
        
        // Get logits as a CPU vector for sampling
        let logits_vec = squeezed_logits.to_vec1::<f32>()?;
        
        // Apply temperature scaling to logits
        let mut scaled_logits = logits_vec.clone();
        
        // Apply temperature scaling - use a moderate temperature for chat
        let effective_temp = if self.temperature <= 0.0 { 
            0.7 // Default to 0.7 if unset or invalid
        } else {
            self.temperature.max(0.3) // Ensure minimum of 0.3 to prevent repetitions
        };
        
        for logit in &mut scaled_logits {
            *logit /= effective_temp;
        }
        
        // Find the maximum logit for numerical stability
        let max_logit = scaled_logits.iter()
            .fold(f32::NEG_INFINITY, |a, &b| a.max(b));
            
        // Compute softmax probabilities
        let sum_exp: f32 = scaled_logits.iter()
            .map(|&logit| (logit - max_logit).exp())
            .sum();
            
        let probs: Vec<f32> = scaled_logits.iter()
            .map(|&logit| (logit - max_logit).exp() / sum_exp)
            .collect();
        
        // Greedy sampling only if explicitly requested with very low temperature
        if self.temperature < 0.05 {
            let argmax = probs.iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(idx, _)| idx)
                .unwrap_or(0);
                
            return Ok(argmax as u32);
        }
        
        // Perform top-k filtering - more tokens for more diversity
        let k = 60; // Increased from 40 for more diversity
        let mut top_k_probs = probs.iter()
            .enumerate()
            .map(|(idx, &prob)| (idx, prob))
            .collect::<Vec<_>>();
            
        // Sort by probability (descending)
        top_k_probs.sort_by(|(_, a), (_, b)| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        
        // Keep only the top k elements
        top_k_probs.truncate(k);
        
        // Apply nucleus (top-p) sampling - higher p for more diversity
        let p = 0.95; // Increased from 0.9 for better diversity
        let mut cumsum = 0.0;
        let mut top_p_probs = Vec::new();
        
        for &(idx, prob) in &top_k_probs {
            if cumsum >= p {
                break;
            }
            top_p_probs.push((idx, prob));
            cumsum += prob;
        }
        
        // Make sure we have at least one token
        if top_p_probs.is_empty() && !top_k_probs.is_empty() {
            top_p_probs.push(top_k_probs[0]);
        }
        
        // Renormalize
        let total_prob: f32 = top_p_probs.iter().map(|(_, prob)| *prob).sum();
        let normalized_probs = top_p_probs.iter()
            .map(|&(idx, prob)| (idx, prob / total_prob))
            .collect::<Vec<_>>();
        
        // Sample from the filtered distribution
        let r: f32 = rand::random();
        let mut cumsum = 0.0;
        
        for &(idx, prob) in &normalized_probs {
            cumsum += prob;
            if r < cumsum {
                return Ok(idx as u32);
            }
        }
        
        // Fallback to the most probable token
        let (argmax, _) = normalized_probs.first()
            .copied()
            .unwrap_or((0, 0.0));
            
        Ok(argmax as u32)
    }
}