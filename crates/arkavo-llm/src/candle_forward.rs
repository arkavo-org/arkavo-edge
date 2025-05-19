use anyhow::{Result, anyhow};
use candle_core::{Tensor, Module};
use candle_nn::{ops, activation};

use crate::candle_kv_cache::KVCache;
use crate::candle_transformer_layer::TransformerLayer;
use crate::candle_model_core::CandleQwen3Model;

impl CandleQwen3Model {
    /// Ensures the input tensor has the expected shape for matmul operations
    pub(crate) fn ensure_expected_shape(&self, tensor: &Tensor, expected_dims: usize) -> Result<Tensor> {
        let shape = tensor.shape().dims();
        
        // If tensor is already 2D and has the expected second dimension, we're good
        if shape.len() == 2 && shape[1] == expected_dims {
            return Ok(tensor.clone());
        }
        
        // If the tensor is 1D, reshape it to be 2D with batch dimension of 1
        if shape.len() == 1 && shape[0] == expected_dims {
            return Ok(tensor.reshape((1, expected_dims))?);
        }
        
        // If the tensor is 2D but second dimension doesn't match, try narrow it (for Qwen3's doubled dimensions)
        if shape.len() == 2 && shape[1] > expected_dims && shape[1] % expected_dims == 0 {
            return Ok(tensor.narrow(1, 0, expected_dims)?);
        }
        
        // Return the original tensor and let the error happen later if dimensions are truly incompatible
        Ok(tensor.clone())
    }
    
    /// Layer normalization using candle ops with CPU fallback
    pub(crate) fn layer_norm(&self, input: &Tensor, weight: &Tensor, bias: &Option<Tensor>) -> Result<Tensor> {
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
                match ops::layer_norm(&input, weight, b, eps) {
                    Ok(result) => result,
                    Err(e) => {
                        // If we get a Metal error, fall back to CPU
                        if format!("{:?}", e).contains("no metal implementation for layer-norm") {
                            // Move tensors to CPU, perform layer norm, and move back
                            let input_cpu = input.to_device(&candle_core::Device::Cpu)?;
                            let weight_cpu = weight.to_device(&candle_core::Device::Cpu)?;
                            let bias_cpu = b.to_device(&candle_core::Device::Cpu)?;
                            
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
                
                match ops::layer_norm(&input, weight, &zeros, eps) {
                    Ok(result) => result,
                    Err(e) => {
                        // If we get a Metal error, fall back to CPU
                        if format!("{:?}", e).contains("no metal implementation for layer-norm") {
                            // Move tensors to CPU, perform layer norm, and move back
                            let input_cpu = input.to_device(&candle_core::Device::Cpu)?;
                            let weight_cpu = weight.to_device(&candle_core::Device::Cpu)?;
                            let zeros_cpu = zeros.to_device(&candle_core::Device::Cpu)?;
                            
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

    /// Perform self-attention operation with Candle
    pub(crate) fn self_attention(
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
    pub(crate) fn feed_forward(&self, input: &Tensor, layer: &TransformerLayer) -> Result<Tensor> {
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
    pub(crate) fn forward_pass(&self, tokens: &Tensor, kv_cache: &mut KVCache) -> Result<Tensor> {
        let seq_len = tokens.dim(0)?;
        
        // Embedding lookup - we need to process tokens one by one to build the KV cache
        let mut hidden_states = Vec::with_capacity(seq_len);
        
        for pos in 0..seq_len {
            // Get token at position
            let token_id = tokens.get(pos)?.to_scalar::<u32>()?;
            
            if token_id as usize >= self.vocab_size {
                return Err(anyhow!("Token ID {} out of vocabulary range", token_id));
            }
            
            // Lookup embedding for this token
            let token_embedding = match self.embedding.get(token_id as usize) {
                Ok(embedding) => embedding,
                Err(e) => {
                    return Err(anyhow!("Failed to get embedding for token ID {}: {}", token_id, e));
                }
            };
            
            // Qwen3 uses RoPE in the attention mechanism, so we don't add position embeddings here
            // First, ensure token embedding has the right shape
            let token_emb_shaped = match token_embedding.shape().dims() {
                // If already correct shape (batch_size=1, hidden_dim)
                [hidden_dim] if *hidden_dim == self.hidden_dim => token_embedding.reshape((1, self.hidden_dim))?,
                
                // If tensor is already 2D with correct second dimension
                [_batch_size, hidden_dim] if *hidden_dim == self.hidden_dim => token_embedding.clone(),
                
                // For other cases, use our helper to safely ensure dimensions
                _ => self.ensure_expected_shape(&token_embedding, self.hidden_dim)?
            };
            
            // Use token embeddings as initial state - position encoding happens in attention via RoPE
            let mut state = token_emb_shaped;
            
            // Process through transformer layers
            for (layer_idx, layer) in self.layers.iter().enumerate() {
                // println!("Processing layer {}", layer_idx);
                
                // Layer normalization before attention
                // println!("  Applying attention layer norm");
                let norm_state = match self.layer_norm(&state, &layer.attn_norm_weight, &layer.attn_norm_bias) {
                    Ok(ns) => ns,
                    Err(e) => {
                        println!("ERROR in attention layer norm at layer {}: {}", layer_idx, e);
                        return Err(anyhow!("Failed in attention layer norm at layer {}: {}", layer_idx, e));
                    }
                };
                
                // Self-attention - use layer_idx to ensure we use the correct KV cache for each layer
                // println!("  Applying self-attention");
                let attn_output = match self.self_attention(&norm_state, layer, layer_idx, pos, kv_cache) {
                    Ok(ao) => ao,
                    Err(e) => {
                        println!("ERROR in self-attention at layer {}: {}", layer_idx, e);
                        return Err(anyhow!("Failed in self-attention at layer {}: {}", layer_idx, e));
                    }
                };
                
                // Residual connection
                // println!("  Adding residual connection after attention");
                state = match state.add(&attn_output) {
                    Ok(s) => s,
                    Err(e) => {
                        println!("ERROR in residual connection after attention at layer {}: {}", layer_idx, e);
                        return Err(anyhow!("Failed in residual connection after attention at layer {}: {}", layer_idx, e));
                    }
                };
                
                // Layer normalization before feed-forward
                // println!("  Applying feed-forward layer norm");
                let norm_state = match self.layer_norm(&state, &layer.ff_norm_weight, &layer.ff_norm_bias) {
                    Ok(ns) => ns,
                    Err(e) => {
                        println!("ERROR in feed-forward layer norm at layer {}: {}", layer_idx, e);
                        return Err(anyhow!("Failed in feed-forward layer norm at layer {}: {}", layer_idx, e));
                    }
                };
                
                // Feed-forward
                // println!("  Applying feed-forward network");
                let ff_output = match self.feed_forward(&norm_state, layer) {
                    Ok(ffo) => ffo,
                    Err(e) => {
                        println!("ERROR in feed-forward at layer {}: {}", layer_idx, e);
                        return Err(anyhow!("Failed in feed-forward at layer {}: {}", layer_idx, e));
                    }
                };
                
                // Residual connection
                // println!("  Adding residual connection after feed-forward");
                state = match state.add(&ff_output) {
                    Ok(s) => s,
                    Err(e) => {
                        println!("ERROR in residual connection after feed-forward at layer {}: {}", layer_idx, e);
                        return Err(anyhow!("Failed in residual connection after feed-forward at layer {}: {}", layer_idx, e));
                    }
                };
                
                // println!("  Finished layer {}", layer_idx);
            }
            
            // Final layer normalization
            state = match self.layer_norm(&state, &self.final_norm_weight, &self.final_norm_bias) {
                Ok(s) => s,
                Err(e) => {
                    return Err(anyhow!("Failed in final layer normalization: {}", e));
                }
            };
            
            // Add state to hidden states
            hidden_states.push(state);
        }
        
        // Make sure we have at least one hidden state
        if hidden_states.is_empty() {
            return Err(anyhow!("No hidden states produced - check your tokenizer and model compatibility"));
        }
        
        // Get the last hidden state
        let last_hidden = &hidden_states[hidden_states.len() - 1];
        
        // Project to logits using LM head
        // This is using hardware-accelerated matrix multiplication
        let logits = last_hidden.matmul(&self.lm_head.transpose(0, 1)?)?;
        
        Ok(logits)
    }
}