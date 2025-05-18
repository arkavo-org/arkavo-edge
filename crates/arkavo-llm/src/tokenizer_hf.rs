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
    
    /// Encode text into token IDs
    pub fn encode(&self, text: &str) -> Result<Vec<u32>> {
        let encoding = self.tokenizer.encode(text, true)
            .map_err(|e| anyhow!("Failed to encode text: {}", e))?;
        
        Ok(encoding.get_ids().to_vec())
    }
    
    /// Decode token IDs back into text
    pub fn decode(&self, tokens: &[u32]) -> Result<String> {
        // Print the first few tokens for debugging
        if !tokens.is_empty() {
            let preview = tokens.iter().take(10).collect::<Vec<_>>();
            eprintln!("DEBUG: First 10 tokens to decode: {:?}", preview);
        }
        
        // For direct token checks
        let contains_im_start = tokens.contains(&151644); // <|im_start|>
        let contains_im_end = tokens.contains(&151645);   // <|im_end|>
        
        // tokenizers takes &[u32] directly, no need to convert to Vec
        let mut text = self.tokenizer.decode(tokens, false) // Set skip_special_tokens to false
            .map_err(|e| anyhow!("Failed to decode tokens: {}", e))?;
        
        // Collect first 100 Unicode characters - always safe
        let preview: String = text.chars().take(100).collect();
        eprintln!("DEBUG: Raw decoded text (first 100 chars): {}", preview);
        
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
        
        // Log the final output - collect first 100 Unicode characters (always safe)
        let post_preview: String = text.chars().take(100).collect();
        eprintln!("DEBUG: Post-processed decoded text (first 100 chars): {}", post_preview);
        
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_tokenizer_encode_decode() -> Result<()> {
        let tokenizer = HfTokenizer::new("models/tokenizer.json")?;
        
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