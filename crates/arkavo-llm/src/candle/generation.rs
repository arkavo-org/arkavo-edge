use anyhow::{Result, anyhow};
use candle_core::Tensor;
use std::time::Instant;

use super::kv_cache::KVCache;
use crate::candle_model::CandleQwen3Model;

impl CandleQwen3Model {
    /// Generates token IDs from the input token IDs
    pub fn generate(&self, input_tokens: &[u32], max_tokens: usize) -> Result<Vec<u32>> {
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
        
        // Convert input tokens to tensor
        let input_tensor = Tensor::new(input_tokens, &self.device)?;
        
        // Get logits for the last token
        let mut logits = self.forward_pass(&input_tensor, &mut kv_cache)?;
        // Generate new tokens auto-regressively
        while tokens_generated < max_tokens {
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
        
        Ok(output)
    }

    /// Sample the next token from the logits using temperature, top-k and top-p (nucleus) sampling
    pub fn sample_next_token(&self, logits: &Tensor) -> Result<u32> {
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

    /// Forward pass with cached key-values (for efficient generation)
    pub(crate) fn forward_pass_with_cache(&self, token: &Tensor, kv_cache: &mut KVCache, position: usize) -> Result<Tensor> {
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
        
        Ok(logits)
    }
}