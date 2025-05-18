use anyhow::{Result, anyhow};
use std::collections::HashMap;
use serde_json::Value;
use regex::Regex;

/// Qwen3 tokenizer implementation
pub struct Qwen3Tokenizer {
    /// In-memory tokenizer data
    embedded_tokenizer_data: &'static [u8],
    
    /// Vocabulary for token encoding/decoding
    vocab: HashMap<String, u32>,
    
    /// Reverse vocabulary for decoding
    id_to_token: HashMap<u32, String>,
    
    /// Byte-pair encoding merge ranks
    merges: HashMap<(String, String), usize>,
    
    /// Special token IDs
    bos_id: u32,
    eos_id: u32,
    pad_id: u32,
    
    /// Regex for tokenization
    pattern: Regex,
}

impl Qwen3Tokenizer {
    /// Creates a new tokenizer using embedded model data
    pub fn new_from_embedded() -> Result<Self> {
        // Access embedded tokenizer data
        use crate::utils::EMBEDDED_TOKENIZER_JSON;
        
        // Parse the tokenizer.json file
        let json_str = std::str::from_utf8(EMBEDDED_TOKENIZER_JSON)
            .map_err(|e| anyhow!("Failed to decode tokenizer JSON: {}", e))?;
            
        let tokenizer_json: Value = serde_json::from_str(json_str)
            .map_err(|e| anyhow!("Failed to parse tokenizer JSON: {}", e))?;
        
        // Extract vocabulary
        let vocab_obj = tokenizer_json["model"]["vocab"]
            .as_object()
            .ok_or_else(|| anyhow!("Missing or invalid vocab in tokenizer JSON"))?;
            
        let mut vocab = HashMap::new();
        let mut id_to_token = HashMap::new();
        
        for (token, id_value) in vocab_obj {
            let id = id_value.as_u64()
                .ok_or_else(|| anyhow!("Invalid token ID for {}", token))? as u32;
            vocab.insert(token.clone(), id);
            id_to_token.insert(id, token.clone());
        }
        
        // Extract merges
        let merges_arr = tokenizer_json["model"]["merges"]
            .as_array()
            .ok_or_else(|| anyhow!("Missing or invalid merges in tokenizer JSON"))?;
            
        let mut merges = HashMap::new();
        for (rank, merge_value) in merges_arr.iter().enumerate() {
            if let Some(merge_str) = merge_value.as_str() {
                // Format 1: "a b"
                let parts: Vec<&str> = merge_str.split(' ').collect();
                if parts.len() != 2 {
                    return Err(anyhow!("Invalid merge format at rank {}: {}", rank, merge_str));
                }
                merges.insert((parts[0].to_string(), parts[1].to_string()), rank);
            } else if let Some(merge_array) = merge_value.as_array() {
                // Format 2: ["a", "b"]
                if merge_array.len() != 2 {
                    return Err(anyhow!(
                        "Invalid merge array length at rank {}, expected 2 elements but got {}",
                        rank,
                        merge_array.len()
                    ));
                }
                let first = merge_array[0]
                    .as_str()
                    .ok_or_else(|| anyhow!("Invalid first merge token at rank {}", rank))?;
                let second = merge_array[1]
                    .as_str()
                    .ok_or_else(|| anyhow!("Invalid second merge token at rank {}", rank))?;
                merges.insert((first.to_string(), second.to_string()), rank);
            } else {
                return Err(anyhow!("Invalid merge format at rank {}, expected string or array", rank));
            }
        }
        
        // Extract special tokens with fallbacks
        // Try different naming conventions (bos_token_id, bos_id, etc.)
        let bos_id = tokenizer_json["model"]["bos_token_id"]
            .as_u64()
            .or_else(|| tokenizer_json["model"]["bos_id"].as_u64())
            .or_else(|| {
                // Look for special tokens in the added_tokens section
                if let Some(added_tokens) = tokenizer_json["added_tokens"].as_array() {
                    for token in added_tokens {
                        if let Some(content) = token["content"].as_str() {
                            if content == "<|im_start|>" || content == "<s>" || content == "<|endoftext|>" {
                                return token["id"].as_u64();
                            }
                        }
                    }
                }
                None
            })
            .unwrap_or(151644) as u32; // Default to <|im_start|> token ID
            
        let eos_id = tokenizer_json["model"]["eos_token_id"]
            .as_u64()
            .or_else(|| tokenizer_json["model"]["eos_id"].as_u64())
            .or_else(|| {
                // Look for special tokens in the added_tokens section
                if let Some(added_tokens) = tokenizer_json["added_tokens"].as_array() {
                    for token in added_tokens {
                        if let Some(content) = token["content"].as_str() {
                            if content == "<|im_end|>" || content == "</s>" || content == "<|endoftext|>" {
                                return token["id"].as_u64();
                            }
                        }
                    }
                }
                None
            })
            .unwrap_or(151645) as u32; // Default to <|im_end|> token ID
            
        let pad_id = tokenizer_json["model"]["pad_token_id"]
            .as_u64()
            .or_else(|| tokenizer_json["model"]["pad_id"].as_u64())
            .or_else(|| {
                // Look for special tokens in the added_tokens section
                if let Some(added_tokens) = tokenizer_json["added_tokens"].as_array() {
                    for token in added_tokens {
                        if let Some(content) = token["content"].as_str() {
                            if content == "<|padding|>" || content == "<pad>" {
                                return token["id"].as_u64();
                            }
                        }
                    }
                }
                None
            })
            .unwrap_or(0) as u32; // Default padding ID is typically 0
            
        // Compile the tokenization pattern without look-ahead operators
        // This is a simplified version that achieves similar functionality
        // Original pattern had: r"'s|'t|'re|'ve|'m|'ll|'d| ?\p{L}+| ?\p{N}+| ?[^\s\p{L}\p{N}]+|\s+(?!\S)|\s+"
        let pattern = Regex::new(r"'s|'t|'re|'ve|'m|'ll|'d| ?\p{L}+| ?\p{N}+| ?[^\s\p{L}\p{N}]+|\s+")
            .map_err(|e| anyhow!("Failed to compile tokenizer regex: {}", e))?;
        
        Ok(Self {
            embedded_tokenizer_data: EMBEDDED_TOKENIZER_JSON,
            vocab,
            id_to_token,
            merges,
            bos_id,
            eos_id,
            pad_id,
            pattern,
        })
    }

    /// Encodes the given text into token IDs
    pub fn encode(&self, text: &str) -> Result<Vec<u32>> {
        // Tokenize the text using BPE algorithm
        let mut tokens = vec![self.bos_id]; // Start with BOS token
        
        // Split text into tokens using the pattern
        for token in self.pattern.find_iter(text) {
            let current_token = token.as_str().to_string();
            
            // Apply byte-pair encoding to the token
            let bpe_tokens = self.bpe_encode(&current_token);
            
            // Convert BPE tokens to token IDs
            for token in bpe_tokens {
                if let Some(&id) = self.vocab.get(&token) {
                    tokens.push(id);
                } else {
                    // Handle unknown tokens by encoding each character
                    for c in token.chars() {
                        let char_token = c.to_string();
                        if let Some(&id) = self.vocab.get(&char_token) {
                            tokens.push(id);
                        }
                    }
                }
            }
        }
        
        // End with EOS token
        tokens.push(self.eos_id);
        
        Ok(tokens)
    }

    /// Decodes the given token IDs into text
    pub fn decode(&self, tokens: &[u32]) -> Result<String> {
        // Filter out special tokens
        let filtered_tokens: Vec<_> = tokens.iter()
            .filter(|&&token| token != self.bos_id && token != self.eos_id && token != self.pad_id)
            .collect();
        
        // Convert token IDs to strings
        let mut result = String::new();
        for &token_id in filtered_tokens {
            if let Some(token) = self.id_to_token.get(&token_id) {
                // Handle special whitespace tokens for proper decoding
                if token.starts_with('Ġ') {
                    result.push(' ');
                    result.push_str(&token[1..]);
                } else {
                    result.push_str(token);
                }
            }
        }
        
        Ok(result)
    }
    
    /// Applies byte-pair encoding to a token
    fn bpe_encode(&self, token: &str) -> Vec<String> {
        // First, split the token into individual characters
        let mut chars: Vec<String> = token.chars().map(|c| c.to_string()).collect();
        
        // Apply BPE merges according to the rank order
        while chars.len() > 1 {
            let mut best_merge: Option<(usize, usize)> = None;
            let mut best_rank = usize::MAX;
            
            // Find the best merge
            for i in 0..chars.len() - 1 {
                let pair = (chars[i].clone(), chars[i + 1].clone());
                if let Some(&rank) = self.merges.get(&pair) {
                    if rank < best_rank {
                        best_rank = rank;
                        best_merge = Some((i, i + 1));
                    }
                }
            }
            
            // If no valid merges found, we're done
            if best_merge.is_none() {
                break;
            }
            
            // Apply the best merge
            let (first, second) = best_merge.unwrap();
            let merged = format!("{}{}", chars[first], chars[second]);
            chars[first] = merged;
            chars.remove(second);
        }
        
        chars
    }
}