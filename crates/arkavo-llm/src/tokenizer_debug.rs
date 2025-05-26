use anyhow::Result;
use crate::tokenizer_gguf_core::GgufTokenizer;

/// Analyzes tokenization and provides detailed debug information
/// 
/// This function helps diagnose tokenization issues by:
/// - Showing detailed token mapping for input text
/// - Counting and highlighting UNK tokens
/// - Showing special token handling
/// - Testing round-trip encoding/decoding
pub fn analyze_tokenization_debug(input: &str, tokenizer: &GgufTokenizer) -> Result<String> {
    let mut output = String::new();
    
    // Header section
    output.push_str(&format!("=== TOKENIZER DEBUG ANALYSIS ===\n"));
    output.push_str(&format!("Input text: {}\n", input));
    output.push_str(&format!("Input length: {} characters\n", input.len()));
    output.push_str(&format!("Vocabulary size: {} tokens\n", tokenizer.vocab_size()));
    output.push_str("\n");
    
    // Encode input to tokens
    let tokens = tokenizer.encode(input)?;
    
    // Calculate UNK statistics
    let unk_count = tokens.iter().filter(|&&id| id == 0).count();
    let unk_percentage = if tokens.len() > 0 {
        (unk_count as f64 * 100.0) / tokens.len() as f64 
    } else { 
        0.0 
    };
    
    // Token statistics
    output.push_str(&format!("=== TOKEN STATISTICS ===\n"));
    output.push_str(&format!("Total tokens: {}\n", tokens.len()));
    output.push_str(&format!("UNK tokens: {} ({:.2}%)\n", unk_count, unk_percentage));
    
    if unk_percentage > 10.0 {
        output.push_str("⚠️ WARNING: High rate of unknown tokens detected!\n");
    }
    output.push_str("\n");
    
    // Token details
    output.push_str("=== TOKEN DETAILS ===\n");
    output.push_str("Index\tID\tToken\n");
    output.push_str("-----\t--\t-----\n");
    
    for (i, &id) in tokens.iter().enumerate() {
        let token_str = if id == 0 {
            "<UNK>".to_string()
        } else if let Some(s) = tokenizer.reverse_vocab.get(&id) {
            // Escape special characters for display
            s.replace('\n', "\\n")
             .replace('\t', "\\t")
             .replace('\r', "\\r")
        } else {
            format!("<unknown:{}>", id)
        };
        
        output.push_str(&format!("{}\t{}\t{}\n", i, id, token_str));
    }
    output.push_str("\n");
    
    // Special token check
    output.push_str("=== SPECIAL TOKEN CHECK ===\n");
    let special_tokens = [
        "<|im_start|>", 
        "<|im_end|>", 
        "<|endoftext|>", 
        "<|system|>", 
        "<|user|>", 
        "<|assistant|>",
        "Ġ", // GPT-2 space
        "▁", // SentencePiece space
        "Ċ", // GPT-2 newline
    ];
    
    for &token in &special_tokens {
        if let Some(&id) = tokenizer.vocab.get(token) {
            output.push_str(&format!("✓ Special token '{}' present with ID {}\n", token, id));
        } else {
            output.push_str(&format!("✗ Special token '{}' NOT found in vocabulary\n", token));
        }
    }
    output.push_str("\n");
    
    // Decoding check
    output.push_str("=== DECODING CHECK ===\n");
    let decoded = tokenizer.decode(&tokens)?;
    output.push_str(&format!("Decoded text: {}\n", decoded));
    
    // Check if there are significant differences in content after round-trip
    let input_words: Vec<&str> = input.split_whitespace().collect();
    let decoded_words: Vec<&str> = decoded.split_whitespace().collect();
    
    let input_words_set: std::collections::HashSet<&str> = input_words.iter().copied().collect();
    let decoded_words_set: std::collections::HashSet<&str> = decoded_words.iter().copied().collect();
    
    let missing_words: Vec<&str> = input_words_set.difference(&decoded_words_set).copied().collect();
    
    let has_missing_words = !missing_words.is_empty();
    
    if has_missing_words {
        output.push_str("\n⚠️ Words lost during round-trip encoding/decoding:\n");
        for word in &missing_words {
            output.push_str(&format!("- '{}'\n", word));
        }
    }
    
    // Add diagnosis and recommendations
    output.push_str("\n=== DIAGNOSIS AND RECOMMENDATIONS ===\n");
    
    if unk_percentage > 30.0 {
        output.push_str("CRITICAL ISSUE: Extremely high unknown token rate!\n");
        output.push_str("Possible causes:\n");
        output.push_str("1. Tokenizer vocabulary mismatch with model\n");
        output.push_str("2. Incorrect GGUF model format or corruption\n");
        output.push_str("3. Special tokens missing from vocabulary\n");
        
        output.push_str("\nRecommended actions:\n");
        output.push_str("- Verify GGUF model integrity (re-download if needed)\n");
        output.push_str("- Check that tokenizer is loaded from the same source as model\n");
        output.push_str("- Try using a different tokenizer implementation (HF vs GGUF)\n");
    } else if unk_percentage > 5.0 {
        output.push_str("ISSUE: Elevated unknown token rate.\n");
        output.push_str("Possible causes:\n");
        output.push_str("1. Non-standard characters not in vocabulary\n");
        output.push_str("2. Special tokens or formatting not recognized\n");
        
        output.push_str("\nRecommended actions:\n");
        output.push_str("- Check non-ASCII characters in input\n");
        output.push_str("- Verify special token format (may need angle brackets or specific format)\n");
    } else if has_missing_words {
        output.push_str("ISSUE: Incomplete round-trip encoding/decoding.\n");
        output.push_str("Some content was lost during the tokenization process.\n");
        
        output.push_str("\nRecommended actions:\n");
        output.push_str("- Check decoding logic, especially for special tokens\n");
        output.push_str("- Verify input text normalization\n");
    } else if unk_percentage == 0.0 && !has_missing_words {
        output.push_str("✓ No critical tokenization issues detected.\n");
    }
    
    Ok(output)
}

/// A simplified test function for quick tokenization diagnosis of multiple inputs
pub fn test_tokenization(tokenizer: &GgufTokenizer, inputs: &[&str]) -> Result<String> {
    let mut output = String::new();
    
    output.push_str("=== TOKENIZATION QUICK TEST ===\n\n");
    
    for (i, input) in inputs.iter().enumerate() {
        output.push_str(&format!("Test #{}: \"{}\"\n", i+1, input));
        
        // Encode
        let tokens = tokenizer.encode(input)?;
        
        // UNK stats
        let unk_count = tokens.iter().filter(|&&id| id == 0).count();
        let unk_percentage = if tokens.len() > 0 {
            (unk_count as f64 * 100.0) / tokens.len() as f64 
        } else { 
            0.0 
        };
        
        output.push_str(&format!("- Tokens: {} (UNK: {} - {:.1}%)\n", 
                               tokens.len(), unk_count, unk_percentage));
        
        // First few token IDs
        let max_show = std::cmp::min(10, tokens.len());
        if max_show > 0 {
            output.push_str("- Token IDs: ");
            for &id in tokens.iter().take(max_show) {
                output.push_str(&format!("{} ", id));
            }
            if tokens.len() > max_show {
                output.push_str(&format!("... ({} more)", tokens.len() - max_show));
            }
            output.push_str("\n");
        }
        
        // Decode check
        match tokenizer.decode(&tokens) {
            Ok(decoded) => {
                output.push_str(&format!("- Decoded: \"{}\"\n", decoded));
                
                // Simple match check
                if decoded.contains(input) || input.contains(&decoded) {
                    output.push_str("- Round-trip: ✓ (content preserved)\n");
                } else {
                    output.push_str("- Round-trip: ✗ (content changed)\n");
                }
            },
            Err(e) => {
                output.push_str(&format!("- Decode error: {}\n", e));
            }
        }
        
        output.push_str("\n");
    }
    
    Ok(output)
}

/// Deep comparison of tokenization between multiple inputs
/// Useful for finding subtle differences in how similar inputs are tokenized
pub fn compare_tokenization(tokenizer: &GgufTokenizer, inputs: &[&str]) -> Result<String> {
    if inputs.len() < 2 {
        return Ok("Need at least 2 inputs to compare tokenization".to_string());
    }
    
    let mut output = String::new();
    output.push_str("=== TOKENIZATION COMPARISON ===\n\n");
    
    let mut all_tokens = Vec::new();
    
    // Encode all inputs
    for input in inputs {
        let tokens = tokenizer.encode(input)?;
        all_tokens.push(tokens);
    }
    
    // Compare token counts
    output.push_str("Token counts:\n");
    for (i, tokens) in all_tokens.iter().enumerate() {
        let unk_count = tokens.iter().filter(|&&id| id == 0).count();
        output.push_str(&format!("- Input #{}: {} tokens ({} UNK)\n", 
                              i+1, tokens.len(), unk_count));
    }
    output.push_str("\n");
    
    // Compare specific tokens
    output.push_str("Token comparison:\n");
    
    // Find the longest common prefix
    let min_len = all_tokens.iter().map(|t| t.len()).min().unwrap_or(0);
    
    // Only compare if we have tokens to compare
    if min_len > 0 {
        let mut divergence_pos = min_len;
        
        for pos in 0..min_len {
            let reference = all_tokens[0][pos];
            if all_tokens.iter().skip(1).any(|t| t[pos] != reference) {
                divergence_pos = pos;
                break;
            }
        }
        
        // Show tokens around the divergence point
        let start = divergence_pos.saturating_sub(2);
        let end = std::cmp::min(divergence_pos + 3, min_len);
        
        output.push_str(&format!("First divergence at position {}\n", divergence_pos));
        output.push_str("    Position: ");
        for pos in start..end {
            output.push_str(&format!("{:<8} ", pos));
        }
        output.push_str("\n");
        
        for (i, tokens) in all_tokens.iter().enumerate() {
            output.push_str(&format!("    Input #{}: ", i+1));
            for pos in start..end {
                let id = tokens[pos];
                let token_str = if id == 0 {
                    "<UNK>".to_string()
                } else if let Some(s) = tokenizer.reverse_vocab.get(&id) {
                    if s.len() <= 6 {
                        s.clone()
                    } else {
                        format!("{}...", &s[0..6])
                    }
                } else {
                    format!("?{}", id)
                };
                
                if pos == divergence_pos {
                    output.push_str(&format!("!{:<7} ", token_str));
                } else {
                    output.push_str(&format!("{:<8} ", token_str));
                }
            }
            output.push_str("\n");
        }
    }
    
    Ok(output)
}

/// Extracts vocabulary statistics to help diagnose token issues
pub fn analyze_vocabulary(tokenizer: &GgufTokenizer) -> String {
    let mut output = String::new();
    
    output.push_str("=== VOCABULARY ANALYSIS ===\n\n");
    
    // Vocabulary size
    let vocab_size = tokenizer.vocab_size();
    output.push_str(&format!("Total vocabulary size: {} tokens\n", vocab_size));
    
    // Special token count
    let special_token_count = tokenizer.special_tokens.len();
    output.push_str(&format!("Special tokens: {} defined\n", special_token_count));
    
    // Token length distribution
    let mut length_dist = std::collections::HashMap::new();
    let mut ascii_count = 0;
    let mut non_ascii_count = 0;
    let mut special_char_count = 0;
    
    // Sample up to 1000 tokens for analysis
    let sample_size = std::cmp::min(1000, vocab_size);
    let sample_step = if vocab_size > sample_size { vocab_size / sample_size } else { 1 };
    
    let mut sample_tokens = Vec::new();
    
    // Collect a representative sample
    for (token, _) in tokenizer.vocab.iter().step_by(sample_step).take(sample_size) {
        sample_tokens.push(token);
        
        // Token length (in bytes)
        let token_len = token.len();
        *length_dist.entry(token_len).or_insert(0) += 1;
        
        // Check if ASCII only
        if token.chars().all(|c| c.is_ascii()) {
            ascii_count += 1;
        } else {
            non_ascii_count += 1;
        }
        
        // Check for special characters
        if token.contains('Ġ') || token.contains('▁') || token.contains('Ċ') {
            special_char_count += 1;
        }
    }
    
    // Calculate percentages
    let sample_count = sample_tokens.len();
    let ascii_pct = (ascii_count as f64 * 100.0) / sample_count as f64;
    let non_ascii_pct = (non_ascii_count as f64 * 100.0) / sample_count as f64;
    let special_char_pct = (special_char_count as f64 * 100.0) / sample_count as f64;
    
    output.push_str(&format!("\nToken characteristics (sample of {} tokens):\n", sample_count));
    output.push_str(&format!("- ASCII-only tokens: {:.1}%\n", ascii_pct));
    output.push_str(&format!("- Non-ASCII tokens: {:.1}%\n", non_ascii_pct));
    output.push_str(&format!("- Tokens with special chars (Ġ,▁,Ċ): {:.1}%\n", special_char_pct));
    
    // Token length distribution
    output.push_str("\nToken length distribution:\n");
    let mut lengths: Vec<_> = length_dist.iter().collect();
    lengths.sort_by_key(|&(k, _)| *k);
    
    for (len, count) in lengths {
        let pct = (*count as f64 * 100.0) / sample_count as f64;
        output.push_str(&format!("- {} bytes: {:.1}% ({} tokens)\n", len, pct, count));
    }
    
    // Tokenizer type detection
    output.push_str("\nTokenizer type detection:\n");
    
    if special_char_pct > 5.0 {
        if tokenizer.vocab.contains_key("Ġ") {
            output.push_str("✓ Appears to be a GPT-2 style tokenizer (uses Ġ for spaces)\n");
        } else if tokenizer.vocab.contains_key("▁") {
            output.push_str("✓ Appears to be a SentencePiece tokenizer (uses ▁ for spaces)\n");
        } else {
            output.push_str("✓ Appears to use special characters, but not standard GPT-2/SentencePiece markers\n");
        }
    } else {
        if ascii_pct > 90.0 {
            output.push_str("✓ Appears to be a simple character-level or word-level tokenizer\n");
        } else {
            output.push_str("✓ Appears to be a BPE or WordPiece tokenizer without special markers\n");
        }
    }
    
    output
}

/// Helper function to add debug IDs to string entries
/// This can help visualize tokenization boundaries
pub fn add_debug_markers(text: &str) -> String {
    let mut result = String::with_capacity(text.len() * 2);
    for (i, c) in text.chars().enumerate() {
        if i % 5 == 0 {
            result.push_str(&format!("[{}]", i));
        }
        result.push(c);
    }
    result
}