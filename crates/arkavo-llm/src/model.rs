use anyhow::{Result, anyhow};
use ndarray::{Array, Array1, Array2, Array3, Axis};
use safetensors::SafeTensors;
use std::collections::HashMap;
use crate::{Qwen3Config, LlmError};

// Transformer layer structure for Qwen3
struct TransformerLayer {
    // Self-attention weights
    query_weight: Array2<f32>,
    key_weight: Array2<f32>,
    value_weight: Array2<f32>,
    output_weight: Array2<f32>,
    
    // Layer normalization parameters
    attn_norm_weight: Array1<f32>,
    attn_norm_bias: Array1<f32>,
    
    // Feed-forward network weights
    ff_inter_weight: Array2<f32>,
    ff_inter_bias: Array1<f32>,
    ff_output_weight: Array2<f32>,
    ff_output_bias: Array1<f32>,
    
    // Final layer normalization parameters
    ff_norm_weight: Array1<f32>,
    ff_norm_bias: Array1<f32>,
}

/// Qwen3 model implementation for Qwen3-0.6B
pub struct Qwen3Model {
    /// Whether to use GPU for inference
    use_gpu: bool,
    
    /// In-memory model data
    embedded_model_data: &'static [u8],
    
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
    
    /// Transformer weights
    embedding: Array2<f32>,
    position_embedding: Array2<f32>,
    layers: Vec<TransformerLayer>,
    final_norm_weight: Array1<f32>,
    final_norm_bias: Array1<f32>,
    lm_head: Array2<f32>,
    
    /// Tensor mapping
    tensors: HashMap<String, Array<f32, ndarray::IxDyn>>,
}

impl Qwen3Model {
    /// Creates a new model from the given configuration
    pub fn new(config: &Qwen3Config) -> Result<Self> {
        // Check if model path exists (for non-embedded models)
        if !config.model_path.starts_with("memory://") {
            return Err(anyhow!(LlmError::ModelLoadError(
                format!("Only memory:// models are supported in this implementation")
            )));
        }
        
        // This is a placeholder implementation that creates a non-functional model
        let hidden_dim = 1024;
        let num_layers = 12;
        let num_heads = 16;
        let vocab_size = 151936;
        let head_dim = hidden_dim / num_heads;
        
        // Create empty tensors for the model
        let tensor_map = HashMap::new();
        let embedding = Array2::zeros((vocab_size, hidden_dim));
        let position_embedding = Array2::zeros((2048, hidden_dim));
        let final_norm_weight = Array1::ones(hidden_dim);
        let final_norm_bias = Array1::zeros(hidden_dim);
        let lm_head = Array2::zeros((vocab_size, hidden_dim));
        
        // Create empty transformer layers
        let layers = Vec::new();
        
        Ok(Self {
            use_gpu: config.use_gpu,
            embedded_model_data: &[],
            is_loaded: false,
            hidden_dim,
            num_layers,
            num_heads,
            head_dim,
            vocab_size,
            temperature: config.temperature,
            embedding,
            position_embedding,
            layers,
            final_norm_weight,
            final_norm_bias,
            lm_head,
            tensors: tensor_map,
        })
    }
        
    /// Creates a new Qwen3Model using embedded model data
    #[cfg(feature = "embedded_model")]
    pub fn new_from_embedded(config: &Qwen3Config) -> Result<Self> {
        // Access embedded model data
        use crate::utils::EMBEDDED_MODEL_SAFETENSORS;
        use crate::utils::EMBEDDED_CONFIG_JSON;
        
        // Parse config file to get model architecture parameters
        let config_str = std::str::from_utf8(EMBEDDED_CONFIG_JSON)
            .map_err(|e| anyhow!("Failed to decode config JSON: {}", e))?;
        
        let config_json: serde_json::Value = serde_json::from_str(config_str)
            .map_err(|e| anyhow!("Failed to parse config JSON: {}", e))?;
        
        let hidden_dim = config_json["hidden_size"]
            .as_u64()
            .unwrap_or(1024) as usize;
            
        let num_layers = config_json["num_hidden_layers"]
            .as_u64()
            .unwrap_or(12) as usize;
            
        let num_heads = config_json["num_attention_heads"]
            .as_u64()
            .unwrap_or(16) as usize;
            
        let vocab_size = config_json["vocab_size"]
            .as_u64()
            .unwrap_or(151936) as usize;
            
        let head_dim = hidden_dim / num_heads;
        
        // Load model weights from safetensors format
        let tensors = SafeTensors::deserialize(EMBEDDED_MODEL_SAFETENSORS)
            .map_err(|e| anyhow!("Failed to deserialize model: {}", e))?;
            
        // Create tensor mapping for more efficient access
        let mut tensor_map = HashMap::new();
        
        // Extract all tensors
        for tensor_name in tensors.names() {
            let tensor_view = tensors.tensor(tensor_name)?;
            let shape = tensor_view.shape().to_vec();
            
            // Only load f32 tensors
            if tensor_view.dtype() == safetensors::Dtype::F32 {
                // Get data as bytes
                let data = tensor_view.data();
                // Convert bytes to f32 slice
                let f32_data = unsafe {
                    std::slice::from_raw_parts(
                        data.as_ptr() as *const f32,
                        data.len() / std::mem::size_of::<f32>(),
                    )
                };
                
                let ndarray = match shape.len() {
                    1 => Array::from_shape_vec(shape[0], f32_data.to_vec())
                        .map_err(|e| anyhow!("Failed to create 1D array: {}", e))?
                        .into_dyn(),
                    2 => Array::from_shape_vec((shape[0], shape[1]), f32_data.to_vec())
                        .map_err(|e| anyhow!("Failed to create 2D array: {}", e))?
                        .into_dyn(),
                    3 => Array::from_shape_vec((shape[0], shape[1], shape[2]), f32_data.to_vec())
                        .map_err(|e| anyhow!("Failed to create 3D array: {}", e))?
                        .into_dyn(),
                    _ => return Err(anyhow!("Unsupported tensor shape: {:?}", shape)),
                };
                
                tensor_map.insert(tensor_name.to_string(), ndarray);
            }
        }
        
        // Extract embeddings
        let embedding = tensor_map
            .get("transformer.wte.weight")
            .ok_or_else(|| anyhow!("Missing embedding weights"))?
            .clone()
            .into_dimensionality::<ndarray::Ix2>()?;
            
        let position_embedding = tensor_map
            .get("transformer.wpe.weight")
            .ok_or_else(|| anyhow!("Missing position embedding weights"))?
            .clone()
            .into_dimensionality::<ndarray::Ix2>()?;
            
        // Extract final layer norm weights
        let final_norm_weight = tensor_map
            .get("transformer.ln_f.weight")
            .ok_or_else(|| anyhow!("Missing final layer norm weights"))?
            .clone()
            .into_dimensionality::<ndarray::Ix1>()?;
            
        let final_norm_bias = tensor_map
            .get("transformer.ln_f.bias")
            .ok_or_else(|| anyhow!("Missing final layer norm bias"))?
            .clone()
            .into_dimensionality::<ndarray::Ix1>()?;
            
        // Extract LM head weights (usually tied to embedding weights)
        let lm_head = match tensor_map.get("lm_head.weight") {
            Some(lm) => lm.clone().into_dimensionality::<ndarray::Ix2>()?,
            None => embedding.clone(), // Fall back to embedding if LM head not defined
        };
            
        // Load transformer layers
        let mut layers = Vec::new();
        for i in 0..num_layers {
            let layer_prefix = format!("transformer.h.{}", i);
            
            // Extract attention weights
            let query_weight = tensor_map
                .get(&format!("{}.attn.c_attn.weight", layer_prefix))
                .ok_or_else(|| anyhow!("Missing query weight for layer {}", i))?
                .clone()
                .into_dimensionality::<ndarray::Ix2>()?;
                
            let key_weight = tensor_map
                .get(&format!("{}.attn.c_attn.weight", layer_prefix))
                .ok_or_else(|| anyhow!("Missing key weight for layer {}", i))?
                .clone()
                .into_dimensionality::<ndarray::Ix2>()?;
                
            let value_weight = tensor_map
                .get(&format!("{}.attn.c_attn.weight", layer_prefix))
                .ok_or_else(|| anyhow!("Missing value weight for layer {}", i))?
                .clone()
                .into_dimensionality::<ndarray::Ix2>()?;
                
            let output_weight = tensor_map
                .get(&format!("{}.attn.c_proj.weight", layer_prefix))
                .ok_or_else(|| anyhow!("Missing output weight for layer {}", i))?
                .clone()
                .into_dimensionality::<ndarray::Ix2>()?;
                
            // Extract layer norm parameters
            let attn_norm_weight = tensor_map
                .get(&format!("{}.ln_1.weight", layer_prefix))
                .ok_or_else(|| anyhow!("Missing attention norm weight for layer {}", i))?
                .clone()
                .into_dimensionality::<ndarray::Ix1>()?;
                
            let attn_norm_bias = tensor_map
                .get(&format!("{}.ln_1.bias", layer_prefix))
                .ok_or_else(|| anyhow!("Missing attention norm bias for layer {}", i))?
                .clone()
                .into_dimensionality::<ndarray::Ix1>()?;
                
            // Extract feed-forward weights
            let ff_inter_weight = tensor_map
                .get(&format!("{}.mlp.c_fc.weight", layer_prefix))
                .ok_or_else(|| anyhow!("Missing FF intermediate weight for layer {}", i))?
                .clone()
                .into_dimensionality::<ndarray::Ix2>()?;
                
            let ff_inter_bias = tensor_map
                .get(&format!("{}.mlp.c_fc.bias", layer_prefix))
                .ok_or_else(|| anyhow!("Missing FF intermediate bias for layer {}", i))?
                .clone()
                .into_dimensionality::<ndarray::Ix1>()?;
                
            let ff_output_weight = tensor_map
                .get(&format!("{}.mlp.c_proj.weight", layer_prefix))
                .ok_or_else(|| anyhow!("Missing FF output weight for layer {}", i))?
                .clone()
                .into_dimensionality::<ndarray::Ix2>()?;
                
            let ff_output_bias = tensor_map
                .get(&format!("{}.mlp.c_proj.bias", layer_prefix))
                .ok_or_else(|| anyhow!("Missing FF output bias for layer {}", i))?
                .clone()
                .into_dimensionality::<ndarray::Ix1>()?;
                
            // Extract feed-forward layer norm parameters
            let ff_norm_weight = tensor_map
                .get(&format!("{}.ln_2.weight", layer_prefix))
                .ok_or_else(|| anyhow!("Missing FF norm weight for layer {}", i))?
                .clone()
                .into_dimensionality::<ndarray::Ix1>()?;
                
            let ff_norm_bias = tensor_map
                .get(&format!("{}.ln_2.bias", layer_prefix))
                .ok_or_else(|| anyhow!("Missing FF norm bias for layer {}", i))?
                .clone()
                .into_dimensionality::<ndarray::Ix1>()?;
                
            // Create transformer layer
            layers.push(TransformerLayer {
                query_weight,
                key_weight,
                value_weight,
                output_weight,
                attn_norm_weight,
                attn_norm_bias,
                ff_inter_weight,
                ff_inter_bias,
                ff_output_weight,
                ff_output_bias,
                ff_norm_weight,
                ff_norm_bias,
            });
        }
        
        Ok(Self {
            use_gpu: config.use_gpu,
            embedded_model_data: EMBEDDED_MODEL_SAFETENSORS,
            is_loaded: true,
            hidden_dim,
            num_layers,
            num_heads,
            head_dim,
            vocab_size,
            temperature: config.temperature,
            embedding,
            position_embedding,
            layers,
            final_norm_weight,
            final_norm_bias,
            lm_head,
            tensors: tensor_map,
        })
    }

    /// Generates token IDs from the input token IDs
    pub fn generate(&self, input_tokens: &[u32], max_tokens: usize) -> Result<Vec<u32>> {
        if !self.is_loaded {
            return Err(anyhow::anyhow!("Model not loaded"));
        }
        
        // Start with the input tokens
        let mut output = input_tokens.to_vec();
        
        // Loop through and generate new tokens one by one
        let mut tokens_generated = 0;
        
        // Keep track of key/value cache for efficient inference
        let mut kv_cache = self.initialize_kv_cache(input_tokens.len());
        
        // Run forward pass on all input tokens to build the initial KV cache
        let mut logits = self.forward_pass(input_tokens, &mut kv_cache)?;
        
        // Generate new tokens auto-regressively
        while tokens_generated < max_tokens {
            // Sample next token based on logits and temperature
            let next_token = self.sample_next_token(&logits)?;
            
            // Stop if we hit the end of sequence token
            if next_token == 2 { // EOS token
                break;
            }
            
            // Add token to output
            output.push(next_token);
            tokens_generated += 1;
            
            // Update logits by running forward pass with just the new token
            logits = self.forward_pass_with_cache(&[next_token], &mut kv_cache)?;
        }
        
        Ok(output)
    }
    
    /// Initialize the key-value cache for efficient autoregressive generation
    fn initialize_kv_cache(&self, seq_len: usize) -> Vec<(Array3<f32>, Array3<f32>)> {
        // Initialize key-value cache for each layer
        // Each cache entry is a pair of tensors: (key_cache, value_cache)
        let mut kv_cache = Vec::with_capacity(self.num_layers);
        
        for _ in 0..self.num_layers {
            // Initialize with zeros
            // Shape: [batch_size, num_heads, seq_len, head_dim]
            let key_cache = Array3::zeros((self.num_heads, seq_len, self.head_dim));
            let value_cache = Array3::zeros((self.num_heads, seq_len, self.head_dim));
            
            kv_cache.push((key_cache, value_cache));
        }
        
        kv_cache
    }
    
    /// Layer normalization function
    fn layer_norm(&self, input: &Array1<f32>, weight: &Array1<f32>, bias: &Array1<f32>) -> Array1<f32> {
        let eps = 1e-5;
        
        // Calculate mean and variance
        let mean = input.mean().unwrap_or(0.0);
        let var = input.iter().map(|&x| (x - mean).powi(2)).sum::<f32>() / input.len() as f32;
        
        // Normalize
        let normalized = input.iter()
            .map(|&x| (x - mean) / (var + eps).sqrt())
            .collect::<Vec<_>>();
            
        // Scale and shift
        let result = normalized.iter()
            .zip(weight.iter())
            .zip(bias.iter())
            .map(|((&n, &w), &b)| n * w + b)
            .collect::<Vec<_>>();
            
        Array1::from_vec(result)
    }
    
    /// Perform self-attention operation
    fn self_attention(&self, 
                    input: &Array1<f32>, 
                    layer: &TransformerLayer, 
                    position: usize,
                    kv_cache: &mut (Array3<f32>, Array3<f32>)) -> Array1<f32> {
        // Project input to query, key, and value
        let query = input.dot(&layer.query_weight);
        
        // Reshape query to [num_heads, head_dim]
        let query_clone = query.clone();
        let query_reshaped = query_clone.into_shape((self.num_heads, self.head_dim))
            .expect("Failed to reshape query");
            
        // Use cached keys and values, or compute them
        let key_reshaped = if position == 0 {
            // For the first token, compute key
            let key = input.dot(&layer.key_weight);
            let key_clone = key.clone();
            let key_reshaped = key_clone.into_shape((self.num_heads, self.head_dim))
                .expect("Failed to reshape key");
                
            // Store in cache
            for h in 0..self.num_heads {
                let key_slice = key_reshaped.slice(ndarray::s![h, ..]);
                kv_cache.0.slice_mut(ndarray::s![h, position, ..])
                    .assign(&key_slice);
            }
            
            key_reshaped
        } else {
            // For subsequent tokens, get from cache
            let mut key_reshaped = Array2::zeros((self.num_heads, self.head_dim));
            for h in 0..self.num_heads {
                key_reshaped.slice_mut(ndarray::s![h, ..])
                    .assign(&kv_cache.0.slice(ndarray::s![h, position, ..]));
            }
            
            key_reshaped
        };
        
        let value_reshaped = if position == 0 {
            // For the first token, compute value
            let value = input.dot(&layer.value_weight);
            let value_clone = value.clone();
            let value_reshaped = value_clone.into_shape((self.num_heads, self.head_dim))
                .expect("Failed to reshape value");
                
            // Store in cache
            for h in 0..self.num_heads {
                let value_slice = value_reshaped.slice(ndarray::s![h, ..]);
                kv_cache.1.slice_mut(ndarray::s![h, position, ..])
                    .assign(&value_slice);
            }
            
            value_reshaped
        } else {
            // For subsequent tokens, get from cache
            let mut value_reshaped = Array2::zeros((self.num_heads, self.head_dim));
            for h in 0..self.num_heads {
                value_reshaped.slice_mut(ndarray::s![h, ..])
                    .assign(&kv_cache.1.slice(ndarray::s![h, position, ..]));
            }
            
            value_reshaped
        };
        
        // Compute attention scores
        let attention_scores = query_reshaped.dot(&key_reshaped.t()) / (self.head_dim as f32).sqrt();
        
        // Apply softmax
        let max_score = attention_scores.fold(f32::NEG_INFINITY, |a, &b| a.max(b));
        let exp_scores: Array2<f32> = attention_scores.mapv(|x| (x - max_score).exp());
        let sum_exp = exp_scores.sum_axis(Axis(1));
        let attention_probs = exp_scores.clone() / sum_exp.slice(ndarray::s![.., ndarray::NewAxis]);
        
        // Apply attention to values
        let context = attention_probs.dot(&value_reshaped);
        
        // Reshape back to original dimensions
        let context_flat = context.into_shape(self.hidden_dim)
            .expect("Failed to reshape context");
            
        // Project back to hidden dimension
        
        
        context_flat.dot(&layer.output_weight)
    }
    
    /// Feed-forward network
    fn feed_forward(&self, input: &Array1<f32>, layer: &TransformerLayer) -> Array1<f32> {
        // First linear layer + GELU activation
        let intermediate = input.dot(&layer.ff_inter_weight) + &layer.ff_inter_bias;
        
        // Apply GELU activation
        let gelu = intermediate.mapv(|x| {
            // GELU approximation: x * 0.5 * (1.0 + tanh(sqrt(2.0/PI) * (x + 0.044715 * x^3)))
            let sqrt_2_over_pi = (2.0 / std::f32::consts::PI).sqrt();
            let x3 = x.powi(3);
            x * 0.5 * (1.0 + ((sqrt_2_over_pi * (x + 0.044715 * x3)).tanh()))
        });
        
        // Second linear layer
        
        
        gelu.dot(&layer.ff_output_weight) + &layer.ff_output_bias
    }
    
    /// Perform the forward pass through the transformer
    fn forward_pass(&self, tokens: &[u32], kv_cache: &mut [(Array3<f32>, Array3<f32>)]) -> Result<Vec<f32>> {
        // Create an array to hold the hidden states for each token
        let mut hidden_states = Vec::with_capacity(tokens.len());
        
        // Process each token
        for (pos, &token) in tokens.iter().enumerate() {
            if token as usize >= self.vocab_size {
                return Err(anyhow!("Token ID {} out of vocabulary range", token));
            }
            
            // Embedding lookup
            let mut state = self.embedding.slice(ndarray::s![token as usize, ..]).to_owned();
            
            // Add position embedding
            let position_embed = self.position_embedding.slice(ndarray::s![pos % 2048, ..]).to_owned();
            state = state + position_embed;
            
            // Process through transformer layers
            for (layer_idx, layer) in self.layers.iter().enumerate() {
                // Layer normalization before attention
                let norm_state = self.layer_norm(&state, &layer.attn_norm_weight, &layer.attn_norm_bias);
                
                // Self-attention
                let attn_output = self.self_attention(&norm_state, layer, pos, &mut kv_cache[layer_idx]);
                
                // Residual connection
                state = state + attn_output;
                
                // Layer normalization before feed-forward
                let norm_state = self.layer_norm(&state, &layer.ff_norm_weight, &layer.ff_norm_bias);
                
                // Feed-forward
                let ff_output = self.feed_forward(&norm_state, layer);
                
                // Residual connection
                state = state + ff_output;
            }
            
            // Final layer normalization
            state = self.layer_norm(&state, &self.final_norm_weight, &self.final_norm_bias);
            
            // Add state to hidden states
            hidden_states.push(state);
        }
        
        // Get the last hidden state
        let last_hidden = hidden_states.last()
            .ok_or_else(|| anyhow!("No hidden states produced"))?;
            
        // Project to logits using LM head
        let logits = last_hidden.dot(&self.lm_head.t()).to_vec();
        
        Ok(logits)
    }
    
    /// Forward pass with cached key-values (for efficient generation)
    fn forward_pass_with_cache(&self, tokens: &[u32], kv_cache: &mut [(Array3<f32>, Array3<f32>)]) -> Result<Vec<f32>> {
        // This is the same as forward_pass but assumes we're only processing one new token
        // Uses and updates the existing KV cache
        
        // For auto-regressive generation, we typically only process one token at a time
        let token = tokens[0];
        
        if token as usize >= self.vocab_size {
            return Err(anyhow!("Token ID {} out of vocabulary range", token));
        }
        
        // Get the current sequence length from the cache
        let seq_len = kv_cache[0].0.shape()[1];
        
        // Embedding lookup
        let mut state = self.embedding.slice(ndarray::s![token as usize, ..]).to_owned();
        
        // Add position embedding
        let position_embed = self.position_embedding.slice(ndarray::s![seq_len % 2048, ..]).to_owned();
        state = state + position_embed;
        
        // Process through transformer layers
        for (layer_idx, layer) in self.layers.iter().enumerate() {
            // Layer normalization before attention
            let norm_state = self.layer_norm(&state, &layer.attn_norm_weight, &layer.attn_norm_bias);
            
            // Self-attention with the new token
            let attn_output = self.self_attention(&norm_state, layer, seq_len, &mut kv_cache[layer_idx]);
            
            // Residual connection
            state = state + attn_output;
            
            // Layer normalization before feed-forward
            let norm_state = self.layer_norm(&state, &layer.ff_norm_weight, &layer.ff_norm_bias);
            
            // Feed-forward
            let ff_output = self.feed_forward(&norm_state, layer);
            
            // Residual connection
            state = state + ff_output;
        }
        
        // Final layer normalization
        state = self.layer_norm(&state, &self.final_norm_weight, &self.final_norm_bias);
        
        // Project to logits using LM head
        let logits = state.dot(&self.lm_head.t()).to_vec();
        
        Ok(logits)
    }
    
    /// Sample the next token from the logits using temperature
    fn sample_next_token(&self, logits: &[f32]) -> Result<u32> {
        // Apply temperature scaling to logits
        let mut scaled_logits = logits.to_vec();
        if self.temperature > 0.0 {
            for logit in &mut scaled_logits {
                *logit /= self.temperature.max(1e-5);
            }
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
            
        // If temperature is very low, just take the argmax
        if self.temperature < 0.1 {
            let argmax = probs.iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(idx, _)| idx)
                .unwrap_or(0);
                
            return Ok(argmax as u32);
        }
        
        // Perform top-k filtering to prevent sampling from very low probability tokens
        let k = 40; // Top-k parameter
        let mut top_k_probs = probs.iter()
            .enumerate()
            .map(|(idx, &prob)| (idx, prob))
            .collect::<Vec<_>>();
            
        // Sort by probability (descending)
        top_k_probs.sort_by(|(_, a), (_, b)| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        
        // Keep only the top k elements
        top_k_probs.truncate(k);
        
        // Renormalize
        let total_prob: f32 = top_k_probs.iter().map(|(_, prob)| *prob).sum();
        let normalized_probs = top_k_probs.iter()
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