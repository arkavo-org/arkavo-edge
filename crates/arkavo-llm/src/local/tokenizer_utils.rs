use tokenizers::Tokenizer;

/// Extract EOS token ID from tokenizer
/// 
/// Tries multiple methods to find the EOS token:
/// 1. Check for explicit EOS token in tokenizer
/// 2. Look for common EOS token strings
/// 3. Fall back to vocab_size - 1
pub fn get_eos_token_id(tokenizer: &Tokenizer) -> Option<u32> {
    // Try to get EOS token from tokenizer's special tokens
    if let Some(eos_id) = tokenizer.token_to_id("</s>") {
        return Some(eos_id);
    }
    
    // Try common EOS token variations
    let eos_variants = ["<|endoftext|>", "<|end|>", "<eos>", "[EOS]", "<|im_end|>"];
    for variant in &eos_variants {
        if let Some(id) = tokenizer.token_to_id(variant) {
            return Some(id);
        }
    }
    
    // Check if tokenizer has get_vocab method
    let vocab_size = tokenizer.get_vocab_size(false);
    if vocab_size > 0 {
        // Many models use the last token as EOS
        Some((vocab_size - 1) as u32)
    } else {
        None
    }
}

/// Get BOS (beginning of sentence) token ID
pub fn get_bos_token_id(tokenizer: &Tokenizer) -> Option<u32> {
    // Try common BOS tokens
    if let Some(bos_id) = tokenizer.token_to_id("<s>") {
        return Some(bos_id);
    }
    
    let bos_variants = ["<|startoftext|>", "<|begin|>", "<bos>", "[BOS]", "<|im_start|>"];
    for variant in &bos_variants {
        if let Some(id) = tokenizer.token_to_id(variant) {
            return Some(id);
        }
    }
    
    // BOS is often token ID 1
    Some(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eos_token_fallback() {
        // This test would require a mock tokenizer
        // For now, we just verify the function compiles
        // Real testing would happen with actual tokenizer instances
    }
}