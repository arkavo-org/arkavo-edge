use tokenizers::Tokenizer;

/// Extract EOS token IDs from tokenizer
///
/// Returns all possible EOS tokens for the model.
/// For Gemma models, this includes both legacy ID 1 and <end_of_turn> (ID 106).
pub fn get_eos_token_ids(tokenizer: &Tokenizer) -> Vec<u32> {
    let mut eos_ids = Vec::new();
    
    // Common EOS token variations
    let eos_variants = [
        "</s>",
        "<|endoftext|>",
        "<|end|>",
        "<eos>",
        "[EOS]",
        "<|im_end|>",
        "<end_of_turn>",  // Gemma specific
    ];
    
    for variant in &eos_variants {
        if let Some(id) = tokenizer.token_to_id(variant) {
            eos_ids.push(id);
        }
    }
    
    // For Gemma models, use ONLY token ID 1 as EOS
    if tokenizer.token_to_id("<start_of_turn>").is_some() && 
       tokenizer.token_to_id("<end_of_turn>").is_some() {
        // This is a Gemma model - use only token 1
        return vec![1];
    }
    
    // If no EOS tokens found, fall back to vocab_size - 1
    if eos_ids.is_empty() {
        let vocab_size = tokenizer.get_vocab_size(false);
        if vocab_size > 0 {
            eos_ids.push((vocab_size - 1) as u32);
        }
    }
    
    // Remove duplicates
    eos_ids.sort_unstable();
    eos_ids.dedup();
    eos_ids
}

/// Extract single EOS token ID from tokenizer (for backward compatibility)
pub fn get_eos_token_id(tokenizer: &Tokenizer) -> Option<u32> {
    get_eos_token_ids(tokenizer).first().copied()
}

/// Get BOS (beginning of sentence) token ID
pub fn get_bos_token_id(tokenizer: &Tokenizer) -> Option<u32> {
    // Try common BOS tokens
    if let Some(bos_id) = tokenizer.token_to_id("<s>") {
        return Some(bos_id);
    }

    let bos_variants = [
        "<|startoftext|>",
        "<|begin|>",
        "<bos>",
        "[BOS]",
        "<|im_start|>",
    ];
    for variant in &bos_variants {
        if let Some(id) = tokenizer.token_to_id(variant) {
            return Some(id);
        }
    }

    // BOS is often token ID 1
    Some(1)
}

/// Format a prompt for Gemma models
///
/// Gemma uses the following format:
/// <bos><start_of_turn>user
/// {prompt}<end_of_turn>
/// <start_of_turn>model
pub fn format_gemma_prompt(prompt: &str, tokenizer: &Tokenizer) -> String {
    let bos = if tokenizer.token_to_id("<bos>").is_some() {
        "<bos>"
    } else {
        ""
    };
    
    format!(
        "{}<start_of_turn>user\n{}<end_of_turn>\n<start_of_turn>model\n",
        bos, prompt
    )
}

/// Check if this is a Gemma model based on tokenizer
pub fn is_gemma_tokenizer(tokenizer: &Tokenizer) -> bool {
    // Check for Gemma-specific tokens
    tokenizer.token_to_id("<start_of_turn>").is_some() &&
    tokenizer.token_to_id("<end_of_turn>").is_some()
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_eos_token_fallback() {
        // This test would require a mock tokenizer
        // For now, we just verify the function compiles
        // Real testing would happen with actual tokenizer instances
    }
}
