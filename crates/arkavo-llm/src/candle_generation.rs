use anyhow::{Result, anyhow};
use candle_core::Tensor;
use std::time::Instant;

use crate::candle_kv_cache::KVCache;
use crate::candle_model_core::CandleQwen3Model;

impl CandleQwen3Model {
    /// Generates token IDs from the input token IDs
    pub fn generate(&self, input_tokens: &[u32], max_tokens: usize) -> Result<Vec<u32>> {
        if !self.is_loaded {
            return Err(anyhow::anyhow!("Model not loaded"));
        }
        
        // For models with empty inputs or very short inputs, just use all input tokens
        // This is safer than trying to find a specific pattern that might not exist
        let prompt_tokens = input_tokens;
        
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
        
        // Check if we have any input tokens
        if input_tokens.is_empty() {
            return Err(anyhow!("Empty input tokens received - tokenizer may be misconfigured"));
        }
        
        // Convert input tokens to tensor
        let input_tensor = match Tensor::new(input_tokens, &self.device) {
            Ok(tensor) => tensor,
            Err(e) => {
                return Err(anyhow!("Failed to create input tensor: {}", e));
            }
        };
        
        // Get logits for the last token - track performance metrics
        let start_time = std::time::Instant::now();
        let mut logits = match self.forward_pass(&input_tensor, &mut kv_cache) {
            Ok(l) => {
                // Log forward pass time in standardized format
                eprintln!("[METRICS] forward={:.2}s", start_time.elapsed().as_secs_f64());
                l
            },
            Err(e) => {
                return Err(anyhow!("Forward pass failed: {}", e));
            }
        };
        
        // Generate tokens up to the requested limit
        
        // Generate new tokens auto-regressively with detailed per-token timing
        let generation_start = std::time::Instant::now();
        let mut total_token_time = 0.0;
        let mut max_token_time: f64 = 0.0;
        let mut min_token_time: f64 = f64::MAX;
        
        while tokens_generated < max_tokens {
            // Time each individual token generation
            let token_start = std::time::Instant::now();
            
            // Sample next token based on logits and temperature
            let next_token = self.sample_next_token(&logits)?;
            
            // Check for EOS token(s)
            // Common EOS tokens in different model architectures
            // Qwen and LLaMA families typically use ID 2 for EOS
            // Some models use ID 1 or ID 0
            // We'll check for several common EOS tokens
            if next_token == 2 || next_token == 1 || next_token == 151645 {
                break;
            }
            
            // Additionally check for any tokens with special meaning
            // 151643 = <|endoftext|> token
            // 151644 = <|im_start|> token
            // 151645 = <|im_end|> token (EOS marker in Qwen 3)
            if next_token == 151645 || (151643..=151650).contains(&next_token) {
                break;
            }
            
            // Add token to output
            output.push(next_token);
            tokens_generated += 1;
            
            // Convert new token to tensor
            let next_token_tensor = Tensor::new(&[next_token], &self.device)?;
            
            // Generate logits for the next token
            // Position is input tokens length + tokens generated so far
            let current_position = input_tokens.len() + tokens_generated;
            logits = self.forward_pass_with_cache(&next_token_tensor, &mut kv_cache, current_position)?;
            
            // Calculate token timing metrics
            let token_duration = token_start.elapsed().as_secs_f64();
            total_token_time += token_duration;
            max_token_time = max_token_time.max(token_duration);
            min_token_time = min_token_time.min(token_duration);
            
            // Log detailed per-token metrics every 5 tokens (to avoid excessive logs)
            if tokens_generated % 5 == 0 || tokens_generated < 5 {
                eprintln!("[TOKEN_METRIC] token={} ctx_pos={} duration={:.3}s", 
                         tokens_generated, current_position, token_duration);
            }
        }
        
        // Log summary token metrics when generation is complete
        if tokens_generated > 0 {
            // Include total generation time in the metrics
            let total_generation_time = generation_start.elapsed().as_secs_f64();
            let avg_token_time = total_token_time / tokens_generated as f64;
            eprintln!("[TOKEN_SUMMARY] tokens={} total={:.2}s avg={:.3}s min={:.3}s max={:.3}s",
                     tokens_generated, total_generation_time, avg_token_time, min_token_time, max_token_time);
        }
        
        // No additional logging here - we'll report metrics at a higher level
        
        Ok(output)
    }

    /// Helper method to get top token predictions (for internal use)
    pub fn debug_top_predictions(&self, logits: &Tensor, top_n: usize) -> Result<()> {
        // Check logits shape and squeeze if needed
        let logits_shape = logits.shape().dims();
        
        // If logits has shape [1, vocab_size], we need to squeeze it to [vocab_size]
        let squeezed_logits = if logits_shape.len() == 2 && logits_shape[0] == 1 {
            logits.squeeze(0)?
        } else {
            logits.clone()
        };
        
        // Get logits as a CPU vector
        let logits_vec = squeezed_logits.to_vec1::<f32>()?;
        
        // Find the top N tokens with highest logits
        let mut indexed_logits: Vec<(usize, f32)> = logits_vec.iter()
            .enumerate()
            .map(|(idx, &val)| (idx, val))
            .collect();
            
        // Sort by logit value in descending order
        indexed_logits.sort_by(|(_, a), (_, b)| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        
        // Take top N
        indexed_logits.truncate(top_n);
        
        // Return without logging - use this data internally for debugging if needed
        Ok(())
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
        // For Qwen3, a temperature of 0.8 often works well
        let effective_temp = if self.temperature <= 0.0 { 
            0.8 // Default to 0.8 if unset or invalid (up from 0.7)
        } else {
            self.temperature.max(0.5) // Ensure minimum of 0.5 to prevent repetitions (up from 0.3)
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
        
        // Perform top-k filtering - adjusted for Qwen3
        let k = 40; // Reduced from 60 for more focused outputs
        
        let mut top_k_probs = probs.iter()
            .enumerate()
            .map(|(idx, &prob)| (idx, prob))
            .collect::<Vec<_>>();
            
        // Sort by probability (descending)
        top_k_probs.sort_by(|(_, a), (_, b)| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        
        // Keep only the top k elements
        top_k_probs.truncate(k);
        
        // Apply nucleus (top-p) sampling - adjusted for better quality 
        let p = 0.9; // Reduced from 0.95 for better focus
        
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