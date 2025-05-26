use anyhow::{Result, anyhow};
use candle_core::{Tensor, Device, DType};
use std::io::Cursor;
use candle_core::quantized::gguf_file;
use std::collections::HashMap;
use crate::candle_model_core::CandleQwen3Model;
use crate::candle_transformer_layer::TransformerLayer;

impl CandleQwen3Model {
    /// Helper method to load tensors from embedded GGUF model bytes
    pub(crate) fn load_from_embedded_gguf(
        hidden_dim: usize,
        num_layers: usize,
        num_heads: usize,
        head_dim: usize,
        vocab_size: usize,
        temperature: f32,
        device: &Device,
    ) -> Result<Self> {
        // Loading embedded model
        
        // Check if we're using GPU acceleration
        let using_gpu = matches!(device, Device::Metal(_) | Device::Cuda(_));
        
        // For GGUF loading, we need to use CPU initially (quantized tensors must be loaded on CPU)
        // After dequantization, we'll move tensors to the GPU if requested
        let _loading_device = Device::Cpu; // Unused but kept for clarity
        
        if using_gpu {
            // Will transfer tensors to appropriate device
        }
        
        // Create a Cursor to read from the embedded bytes as if it were a file
        let mut gguf_data = Cursor::new(crate::EMBEDDED_MODEL);
        
        // Parse the GGUF format header and metadata
        let gguf_content = match gguf_file::Content::read(&mut gguf_data) {
            Ok(content) => {
                // Successfully parsed GGUF header
                content
            },
            Err(e) => {
                // Failed to parse GGUF header
                return Err(anyhow!("Failed to parse GGUF header: {}", e));
            }
        };
        
        // Extract all tensors from the GGUF file
        let model_tensors = Self::extract_tensors_from_gguf(&gguf_content, device, using_gpu)?;
        
        // Create the model structure from the extracted tensors
        Self::build_model_from_tensors(
            model_tensors,
            hidden_dim,
            num_layers,
            num_heads,
            head_dim,
            vocab_size,
            temperature,
            device,
        )
    }
    
    /// Extract all tensors from the GGUF file content
    fn extract_tensors_from_gguf(
        gguf_content: &gguf_file::Content, 
        device: &Device,
        using_gpu: bool,
    ) -> Result<HashMap<String, Tensor>> {
        let mut model_tensors = HashMap::new();
        let loading_device = Device::Cpu; // Used when loading tensors
        
        // Get the tensor metadata and store the tensors
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
                                    Err(_) => {
                                        // Failed to move tensor to GPU
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
        
        Ok(model_tensors)
    }
    
    /// Build a model from extracted tensors
    #[allow(clippy::too_many_arguments)]
    fn build_model_from_tensors(
        model_tensors: HashMap<String, Tensor>,
        hidden_dim: usize,
        num_layers: usize,
        num_heads: usize,
        head_dim: usize,
        vocab_size: usize,
        temperature: f32,
        device: &Device,
    ) -> Result<Self> {
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
                // Creating zero position embeddings
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
                // Using default layer norm weights
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
        let layers = Self::build_transformer_layers(
            &model_tensors,
            num_layers,
            hidden_dim,
            device,
        )?;
        
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
    
    /// Build transformer layers from model tensors
    fn build_transformer_layers(
        model_tensors: &HashMap<String, Tensor>,
        num_layers: usize,
        hidden_dim: usize,
        device: &Device,
    ) -> Result<Vec<TransformerLayer>> {
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
        
        Ok(layers)
    }
}