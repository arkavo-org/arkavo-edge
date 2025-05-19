use anyhow::Result;
use crate::tokenizer_gguf_core::GgufTokenizer;

impl GgufTokenizer {
    /// Tokenize a text string into tokens, implementing BPE encoding algorithm
    /// This handles the core BPE encoding without recursive calls
    pub(crate) fn tokenize_bytes(&self, text: &str) -> Vec<u32> {
        // First, try a direct vocabulary lookup for efficiency
        if let Some(&id) = self.vocab.get(text) {
            return vec![id];
        }
        
        // Convert the text to a sequence of bytes/characters for BPE encoding
        let mut tokens = Vec::new();
        
        // Try to detect what kind of tokenizer we're dealing with
        let is_decimal_byte_tokenizer = self.vocab.contains_key("32") || self.vocab.contains_key("97");
        let _is_char_tokenizer = self.vocab.contains_key(" ") || self.vocab.contains_key("a");
        let is_gpt2_tokenizer = self.vocab.contains_key("Ġ") || self.vocab.contains_key("Ċ");
        let is_sentencepiece_tokenizer = self.vocab.contains_key("▁") || self.vocab.contains_key("▃");
        
        // Start with individual characters/bytes
        let mut parts = Vec::new();
        
        if is_decimal_byte_tokenizer {
            // Use decimal byte representation (e.g., "32" for space)
            for b in text.bytes() {
                let token = format!("{}", b);
                parts.push(token);
            }
        } else if is_gpt2_tokenizer {
            // Use GPT-2 style tokenization with Ġ for space
            for (i, c) in text.chars().enumerate() {
                let token = if i > 0 && c == ' ' {
                    "Ġ".to_string()
                } else if c == '\n' {
                    "Ċ".to_string()
                } else if c == '\t' {
                    "Ĉ".to_string()
                } else {
                    c.to_string()
                };
                parts.push(token);
            }
        } else if is_sentencepiece_tokenizer {
            // SentencePiece style tokenization with ▁ for space
            for c in text.chars() {
                let token = if c == ' ' {
                    "▁".to_string()
                } else if c == '\n' {
                    "\n".to_string() // SentencePiece often keeps newlines as-is
                } else if c == '\t' {
                    "\t".to_string() // SentencePiece often keeps tabs as-is
                } else {
                    c.to_string()
                };
                parts.push(token);
            }
        } else {
            // Default to character-level tokenization
            for c in text.chars() {
                parts.push(c.to_string());
            }
        }
        
        // Apply merges iteratively until no more can be applied
        loop {
            let mut best_pair = None;
            let mut best_idx = None;
            
            // Find the first merge we can apply
            for i in 0..parts.len() - 1 {
                let pair = (parts[i].clone(), parts[i + 1].clone());
                if let Some(merged) = self.merges.get(&pair) {
                    best_pair = Some(merged.clone());
                    best_idx = Some(i);
                    break;
                }
            }
            
            // No more merges to apply
            if best_pair.is_none() {
                break;
            }
            
            // Apply the merge
            let idx = best_idx.unwrap();
            let merged = best_pair.unwrap();
            parts[idx] = merged;
            parts.remove(idx + 1);
        }
        
        // Convert parts to token IDs
        for part in parts {
            if let Some(&id) = self.vocab.get(&part) {
                tokens.push(id);
            } else {
                // Unknown token, add <unk> token (usually ID 0)
                tokens.push(0);
            }
        }
        
        tokens
    }

    /// Encode a text string into token IDs
    pub fn encode(&self, text: &str) -> Result<Vec<u32>> {
        println!("Starting tokenization of text: {} chars", text.len());
        
        // Check if this is an empty string
        if text.is_empty() {
            return Ok(Vec::new());
        }
        
        // First, try a direct lookup in the vocabulary - this is very efficient for
        // common tokens and short sequences that might be in the vocabulary already
        if let Some(&id) = self.vocab.get(text) {
            println!("Direct vocabulary match for full text: using token ID {}", id);
            return Ok(vec![id]);
        }
        
        // Check for common tokens that should be encoded as single tokens
        // Include different representations of whitespace tokens based on tokenizer type
        
        // Check for whitespace tokens with their special representations (Ġ for space, etc.)
        if text == " " {
            // Try common space representations
            let space_variants = ["Ġ", "▁", " "];
            for &variant in &space_variants {
                if let Some(&id) = self.vocab.get(variant) {
                    println!("Found space token (' ') as variant {:?} with ID {}", variant, id);
                    return Ok(vec![id]);
                }
            }
        } else if text == "\n" {
            // Try common newline representations
            let newline_variants = ["Ċ", "\n"];
            for &variant in &newline_variants {
                if let Some(&id) = self.vocab.get(variant) {
                    println!("Found newline token ('\\n') as variant {:?} with ID {}", variant, id);
                    return Ok(vec![id]);
                }
            }
        } else if text == "\t" {
            // Try common tab representations
            let tab_variants = ["Ĉ", "\t"];
            for &variant in &tab_variants {
                if let Some(&id) = self.vocab.get(variant) {
                    println!("Found tab token ('\\t') as variant {:?} with ID {}", variant, id);
                    return Ok(vec![id]);
                }
            }
        } else {
            // For other common tokens
            let common_tokens = ["<|im_start|>", "<|im_end|>", "<|endoftext|>",
                               "<|system|>", "<|user|>", "<|assistant|>"];
            
            for &token in &common_tokens {
                if text == token {
                    if let Some(&id) = self.vocab.get(token) {
                        println!("Direct vocabulary match for special token {:?}: using token ID {}", token, id);
                        return Ok(vec![id]);
                    }
                }
            }
        }
        
        // Also check for frequently used short sequences to optimize common cases
        let short_seq_chars = text.chars().count();
        if short_seq_chars <= 10 {
            // For short sequences (most tokens are <10 chars), try direct lookup first
            // This avoids unnecessary BPE encoding for common words and tokens
            if let Some(&id) = self.vocab.get(text) {
                println!("Direct vocabulary match for short sequence: using token ID {}", id);
                return Ok(vec![id]);
            }
        }
        
        // First check for exact special token matches like <|im_start|>
        // These should be treated as whole tokens, not split
        let special_tokens = [
            "<|im_start|>", "<|im_end|>", "<|endoftext|>",
            "<|system|>", "<|user|>", "<|assistant|>"
        ];
        
        // Track positions in the string where we have special tokens
        let mut special_positions = Vec::new();
        for &token in &special_tokens {
            // Find all occurrences of this special token
            let mut start = 0;
            while start < text.len() {
                if let Some(pos) = text[start..].find(token) {
                    let real_pos = start + pos;
                    special_positions.push((real_pos, real_pos + token.len(), token));
                    start = real_pos + token.len();
                } else {
                    break;
                }
            }
        }
        
        // If there are no special tokens, just use the BPE encode directly
        if special_positions.is_empty() {
            let tokens = self.tokenize_bytes(text);
            
            // Debug logging
            let unk_count = tokens.iter().filter(|&&id| id == 0).count();
            if unk_count > 0 {
                let percentage = (unk_count as f32 * 100.0) / tokens.len() as f32;
                println!("WARNING: Produced {} <unk> tokens out of {} ({:.2}%)", 
                         unk_count, tokens.len(), percentage);
            }
            
            return Ok(tokens);
        }
        
        // Sort by position so we process them in order
        special_positions.sort_by_key(|&(start, _, _)| start);
        
        // Now tokenize the text with special handling for the special tokens
        let mut result = Vec::new();
        let mut pos = 0;
        
        for (start, end, token) in special_positions {
            // Tokenize text before the special token
            if start > pos {
                let text_segment = &text[pos..start];
                let segment_tokens = self.tokenize_bytes(text_segment);
                result.extend(segment_tokens);
            }
            
            // Handle the special token - first try direct vocabulary lookup
            if let Some(&id) = self.vocab.get(token) {
                println!("Found special token {:?} in vocabulary with ID {}", token, id);
                result.push(id);
            } else {
                // If not in vocabulary, use BPE encoding
                println!("Special token {:?} not found in vocabulary, using BPE", token);
                let token_ids = self.tokenize_bytes(token);
                result.extend(token_ids);
            }
            
            pos = end;
        }
        
        // Tokenize any remaining text
        if pos < text.len() {
            let remaining = &text[pos..];
            let remaining_tokens = self.tokenize_bytes(remaining);
            result.extend(remaining_tokens);
        }
        
        println!("Tokenization produced {} tokens", result.len());
        
        // Count UNK tokens
        let unk_count = result.iter().filter(|&&id| id == 0).count();
        if unk_count > 0 {
            let percentage = (unk_count as f32 * 100.0) / result.len() as f32;
            println!("WARNING: Produced {} <unk> tokens out of {} ({:.2}%)", 
                     unk_count, result.len(), percentage);
        }
        
        // Print some tokens for debugging
        println!("First 10 tokens: {:?}", result.iter().take(10).collect::<Vec<_>>());
        if result.len() > 10 {
            println!("Last 5 tokens: {:?}", result.iter().rev().take(5).rev().collect::<Vec<_>>());
        }
        
        Ok(result)
    }
}