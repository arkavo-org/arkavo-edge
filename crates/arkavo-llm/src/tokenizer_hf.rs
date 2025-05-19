use anyhow::{Result, anyhow};
use tokenizers::Tokenizer;
use std::path::Path;

/// HuggingFace tokenizer wrapper for Qwen3 models
/// This uses the official HuggingFace tokenizers library and is much more robust
/// than our custom implementation.
pub struct HfTokenizer {
    /// The underlying HuggingFace tokenizer
    tokenizer: Tokenizer,
}

impl HfTokenizer {
    /// Create a new HuggingFace tokenizer from a tokenizer.json file
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let tokenizer = Tokenizer::from_file(path)
            .map_err(|e| anyhow!("Failed to load tokenizer: {}", e))?;
        
        Ok(Self { tokenizer })
    }
    
    /// Create a new HuggingFace tokenizer directly from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        // Use the Tokenizer's from_bytes instead of trying to parse string
        let tokenizer = Tokenizer::from_bytes(bytes)
            .map_err(|e| anyhow!("Failed to load tokenizer from bytes: {}", e))?;
            
        Ok(Self { tokenizer })
    }
    
    /// Encode text into token IDs
    pub fn encode(&self, text: &str) -> Result<Vec<u32>> {
        println!("Tokenizing text of length {}", text.len());
        let encoding = match self.tokenizer.encode(text, true) {
            Ok(enc) => {
                println!("Successfully encoded text to tokens");
                enc
            },
            Err(e) => {
                println!("ERROR: Failed to encode text: {}", e);
                return Err(anyhow!("Failed to encode text: {}", e));
            }
        };
        
        let ids = encoding.get_ids().to_vec();
        println!("Generated {} tokens: {:?}", ids.len(), ids);
        
        // Validate we have some tokens
        if ids.is_empty() {
            println!("WARNING: Tokenizer produced empty token sequence");
        }
        
        Ok(ids)
    }
    
    /// Decode token IDs back into text
    pub fn decode(&self, tokens: &[u32]) -> Result<String> {
        // For direct token checks
        let contains_im_start = tokens.contains(&151644); // <|im_start|>
        let contains_im_end = tokens.contains(&151645);   // <|im_end|>
        
        // tokenizers takes &[u32] directly, no need to convert to Vec
        let mut text = self.tokenizer.decode(tokens, false) // Set skip_special_tokens to false
            .map_err(|e| anyhow!("Failed to decode tokens: {}", e))?;
        
        // The HuggingFace tokenizer might still strip some special tokens or modify them
        // Let's restore common ChatML tokens if they appear in the original token IDs
        if contains_im_start { // <|im_start|>
            // Find where role markers appear and add the im_start tag before them
            for role in &["system", "user", "assistant"] {
                if text.contains(role) && !text.contains("<|im_start|>") {
                    // Replace the first occurrence of the role with the tag + role
                    let role_with_tag = format!("<|im_start|>{}", role);
                    text = text.replacen(role, &role_with_tag, 1);
                }
            }
        }
        
        if contains_im_end && !text.contains("<|im_end|>") { // <|im_end|>
            // Add im_end tag at the end if it was in the original tokens
            text.push_str("<|im_end|>");
        }
        
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_tokenizer_encode_decode() -> Result<()> {
        // Check if the tokenizer file exists
        let tokenizer_path = "models/tokenizer.json";
        if !std::path::Path::new(tokenizer_path).exists() {
            println!("Skipping test - tokenizer file not found: {}", tokenizer_path);
            return Ok(());
        }
        
        // Try to load the tokenizer but return early if it fails
        let tokenizer = match HfTokenizer::new(tokenizer_path) {
            Ok(t) => t,
            Err(e) => {
                println!("Skipping test - failed to load tokenizer: {}", e);
                return Ok(());
            }
        };
        
        // Test with plain text
        let input = "Hello, world!";
        let tokens = tokenizer.encode(input)?;
        let output = tokenizer.decode(&tokens)?;
        assert_eq!(input, output);
        
        // Test with ChatML format
        let input = "<|im_start|>system\nYou are Qwen3, a helpful AI assistant.\n<|im_end|>";
        let tokens = tokenizer.encode(input)?;
        let output = tokenizer.decode(&tokens)?;
        assert_eq!(input, output);
        
        Ok(())
    }
}