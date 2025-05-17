use anyhow::{Result, anyhow};
use regex::Regex;
use std::borrow::Cow;

// Include the generated tokenizer data from build.rs
include!(concat!(env!("OUT_DIR"), "/tokenizer_static.rs"));

/// A more efficient merge pair with static string references
#[derive(Debug, Clone, Copy)]
struct MergePair<'a> {
    /// First token in the merge pair
    first: &'a str,
    
    /// Second token in the merge pair
    second: &'a str,
    
    /// Rank of this merge in the vocabulary (lower = higher priority)
    rank: usize,
}

/// Qwen3 tokenizer implementation using static pre-compiled data
pub struct StaticQwen3Tokenizer {
    /// Special token IDs
    bos_id: u32,
    eos_id: u32,
    pad_id: u32,
    
    /// Regex for tokenization
    pattern: Regex,
}

impl StaticQwen3Tokenizer {
    /// Creates a new tokenizer using pre-compiled data
    pub fn new() -> Result<Self> {
        // Use pre-compiled special token IDs
        let bos_id = BOS_TOKEN_ID;
        let eos_id = EOS_TOKEN_ID;
        let pad_id = PAD_TOKEN_ID;
        
        // Compile the tokenization pattern (no lookaheads for better compatibility)
        let pattern = Regex::new(r"'s|'t|'re|'ve|'m|'ll|'d| ?\p{L}+| ?\p{N}+| ?[^\s\p{L}\p{N}]+|\s+")
            .map_err(|e| anyhow!("Failed to compile tokenizer regex: {}", e))?;
        
        Ok(Self {
            bos_id,
            eos_id,
            pad_id,
            pattern,
        })
    }

    /// Encodes the given text into token IDs
    pub fn encode(&self, text: &str) -> Result<Vec<u32>> {
        // Get the static vocabulary
        let vocab = get_vocab();
        
        // Pre-allocate token vector with a reasonable capacity
        let mut tokens = Vec::with_capacity(text.len() / 4 + 2);
        
        // Start with BOS token
        tokens.push(self.bos_id);
        
        // Split text into tokens using the pattern
        for token in self.pattern.find_iter(text) {
            let current_token = token.as_str();
            
            // Apply byte-pair encoding to the token
            let bpe_tokens = self.bpe_encode(current_token);
            
            // Convert BPE tokens to token IDs
            for token in bpe_tokens {
                // Try to find token in vocab
                if let Some(&id) = vocab.get(token.as_ref()) {
                    tokens.push(id);
                } else {
                    // Handle unknown tokens by encoding each character
                    let mut found_any = false;
                    for c in token.chars() {
                        let c_str = c.to_string();
                        if let Some(&id) = vocab.get(c_str.as_str()) {
                            tokens.push(id);
                            found_any = true;
                        }
                    }
                    
                    // If we couldn't tokenize even character by character, use an UNK token
                    if !found_any && !token.is_empty() {
                        // Use the first token ID as UNK for simplicity
                        // In a real implementation, you'd have a proper UNK token ID
                        tokens.push(0);
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
        // Get the static id to token mapping
        let id_to_token = get_id_to_token();
        
        // Filter out special tokens and estimate result size
        let filtered_tokens: Vec<_> = tokens.iter()
            .filter(|&&token| token != self.bos_id && token != self.eos_id && token != self.pad_id)
            .collect();
        
        // Pre-allocate result string with a reasonable capacity
        let mut result = String::with_capacity(filtered_tokens.len() * 5);
        
        // Convert token IDs to strings
        for &token_id in &filtered_tokens {
            if let Some(token) = id_to_token.get(&token_id) {
                // Decode the token, which may be surrounded by quotes
                let token_str = if token.starts_with('"') && token.ends_with('"') && token.len() >= 2 {
                    &token[1..token.len()-1]
                } else {
                    token
                };
                
                // Handle special whitespace tokens for proper decoding
                if token_str.starts_with('Ġ') {
                    result.push(' ');
                    result.push_str(&token_str[1..]);
                } else {
                    result.push_str(token_str);
                }
            }
        }
        
        Ok(result)
    }
    
    /// Applies byte-pair encoding to a token
    fn bpe_encode<'a>(&self, token: &'a str) -> Vec<Cow<'a, str>> {
        // Don't allocate for empty tokens
        if token.is_empty() {
            return Vec::new();
        }
        
        // First, split the token into individual characters
        let mut parts: Vec<Cow<'a, str>> = token.chars().map(|c| Cow::Owned(c.to_string())).collect();
        
        // Short-circuit for single character tokens
        if parts.len() <= 1 {
            return parts;
        }
        
        // Get the static list of merge pairs
        let merges = MERGES;
        
        // Continue merging until no more merges can be applied
        'outer: while parts.len() > 1 {
            let mut best_merge: Option<(usize, usize)> = None;
            let mut best_rank = usize::MAX;
            
            // Find the best merge
            for i in 0..parts.len() - 1 {
                let first = parts[i].as_ref();
                let second = parts[i+1].as_ref();
                
                // Look for matching merge in the static merge list
                for &(merge_first, merge_second, rank) in merges {
                    if first == merge_first && second == merge_second && rank < best_rank {
                        best_rank = rank;
                        best_merge = Some((i, i + 1));
                        
                        // Optimization: if we find the highest priority merge (rank 0), 
                        // we can stop searching
                        if rank == 0 {
                            break;
                        }
                    }
                }
            }
            
            // If no valid merges found, we're done
            if best_merge.is_none() {
                break 'outer;
            }
            
            // Apply the best merge
            let (first_idx, second_idx) = best_merge.unwrap();
            let merged = format!("{}{}", parts[first_idx], parts[second_idx]);
            parts[first_idx] = Cow::Owned(merged);
            parts.remove(second_idx);
        }
        
        parts
    }
}