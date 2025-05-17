use anyhow::{Result, anyhow};
use regex::Regex;
use std::collections::HashMap;

/// A much simplified embedded tokenizer that doesn't require build-time processing
/// This provides minimal but functional tokenization for the Qwen3 model
pub struct EmbeddedQwen3Tokenizer {
    /// Byte-pair encoding merge ranks
    merges: HashMap<(String, String), usize>,
    
    /// Vocabulary of common tokens
    vocab: HashMap<String, u32>,
    
    /// Reverse mapping
    id_to_token: HashMap<u32, String>,
    
    /// Special token IDs
    bos_id: u32,
    eos_id: u32,
    pad_id: u32,
    
    /// Regex for tokenization
    pattern: Regex,
}

impl EmbeddedQwen3Tokenizer {
    /// Creates a new embedded tokenizer with hard-coded essential tokens
    pub fn new() -> Result<Self> {
        // Special token IDs for Qwen3
        let bos_id = 151644; // <|im_start|>
        let eos_id = 151645; // <|im_end|>
        let pad_id = 0;      // padding token
        
        // Create a minimal vocabulary with essential tokens
        let mut vocab = HashMap::new();
        let mut id_to_token = HashMap::new();
        
        // Add basic characters (ASCII first)
        for (i, c) in " abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.,;:!?-()[]{}\"'`".chars().enumerate() {
            let id = i as u32;
            vocab.insert(c.to_string(), id);
            id_to_token.insert(id, c.to_string());
        }
        
        // Add special Qwen3 tokens
        vocab.insert("<|im_start|>".to_string(), bos_id);
        vocab.insert("<|im_end|>".to_string(), eos_id);
        vocab.insert("<|padding|>".to_string(), pad_id);
        
        id_to_token.insert(bos_id, "<|im_start|>".to_string());
        id_to_token.insert(eos_id, "<|im_end|>".to_string());
        id_to_token.insert(pad_id, "<|padding|>".to_string());
        
        // Add common word pieces
        let common_tokens = [
            // Common English words
            ("the", 100), ("a", 101), ("and", 102), ("to", 103), ("is", 104),
            ("in", 105), ("that", 106), ("it", 107), ("for", 108), ("you", 109),
            ("was", 110), ("with", 111), ("on", 112), ("as", 113), ("are", 114),
            ("at", 115), ("be", 116), ("this", 117), ("have", 118), ("from", 119),
            ("or", 120), ("had", 121), ("by", 122), ("not", 123), ("but", 124),
            ("what", 125), ("all", 126), ("were", 127), ("we", 128), ("when", 129),
            ("your", 130), ("can", 131), ("said", 132), ("there", 133), ("use", 134),
            ("an", 135), ("each", 136), ("which", 137), ("do", 138), ("how", 139),
            ("if", 140), ("will", 141), ("up", 142), ("other", 143), ("about", 144),
            
            // Common code tokens
            ("function", 1000), ("return", 1001), ("const", 1002), ("let", 1003), ("var", 1004),
            ("if", 1005), ("else", 1006), ("for", 1007), ("while", 1008), ("class", 1009),
            ("int", 1010), ("string", 1011), ("bool", 1012), ("true", 1013), ("false", 1014),
            ("null", 1015), ("undefined", 1016), ("import", 1017), ("export", 1018), ("from", 1019),
            ("public", 1020), ("private", 1021), ("protected", 1022), ("static", 1023), ("new", 1024),
            
            // Special Qwen tokens with Ġ prefix (space)
            ("Ġthe", 2000), ("Ġa", 2001), ("Ġand", 2002), ("Ġto", 2003), ("Ġis", 2004),
            ("Ġin", 2005), ("Ġthat", 2006), ("Ġit", 2007), ("Ġfor", 2008), ("Ġyou", 2009),
        ];
        
        for (token, id) in common_tokens.iter() {
            vocab.insert(token.to_string(), *id);
            id_to_token.insert(*id, token.to_string());
        }
        
        // Minimal set of BPE merges for common token pairs
        let mut merges = HashMap::new();
        let common_merges = [
            // Character pairs
            ("t", "h", 0), ("h", "e", 1), ("i", "n", 2), ("e", "r", 3),
            ("a", "n", 4), ("o", "r", 5), ("t", "i", 6), ("e", "s", 7),
            ("o", "n", 8), ("a", "t", 9), ("e", "n", 10), ("n", "d", 11),
            ("i", "s", 12), ("i", "n", 13), ("r", "e", 14), ("t", "e", 15),
            
            // Common word merges
            ("th", "e", 100), ("a", "nd", 101), ("in", "g", 102), ("re", "s", 103),
            ("er", "s", 104), ("t", "ion", 105), ("pro", "gram", 106), ("com", "put", 107),
        ];
        
        for (i, (first, second, _)) in common_merges.iter().enumerate() {
            merges.insert((first.to_string(), second.to_string()), i);
        }
        
        // Compile tokenization pattern (without lookaheads)
        let pattern = Regex::new(r"'s|'t|'re|'ve|'m|'ll|'d| ?\p{L}+| ?\p{N}+| ?[^\s\p{L}\p{N}]+|\s+")
            .map_err(|e| anyhow!("Failed to compile tokenizer regex: {}", e))?;
        
        Ok(Self {
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
                        } else {
                            // Use a placeholder for truly unknown characters
                            tokens.push(3); // Common unknown token ID
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
            let mut best_rank = std::usize::MAX;
            
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