use anyhow::Result;
use std::collections::HashMap;
use crate::tokenizer_gguf_core::GgufTokenizer;

impl GgufTokenizer {
    /// Decode token IDs back into text
    pub fn decode(&self, token_ids: &[u32]) -> Result<String> {
        // Decode tokens to text
        let mut result = String::new();
        
        // Special token IDs to handle differently
        let special_ids: HashMap<u32, &str> = self.special_tokens.iter()
            .map(|(name, &id)| (id, name.as_str()))
            .collect();
        
        // Also filter high token IDs that might be special markers for Qwen3
        // 151643 = Assistant, 151644 = Human, 151645 = End, etc.
        let qwen_special_range_start = 151643;
        let qwen_special_range_end = 151650;
        
        // Track the current role for better formatting
        let mut current_role: Option<&str> = None;
        
        // Collect ChatML blocks - this helps format the output better
        let mut blocks: Vec<(Option<&str>, String)> = Vec::new();
        let mut current_block_text = String::new();
        
        // Process tokens in a smarter way to handle ChatML blocks
        let mut i = 0;
        while i < token_ids.len() {
            let token_id = token_ids[i];
            
            // Check if this is a special token
            let is_special = special_ids.contains_key(&token_id) ||
                             (token_id >= qwen_special_range_start && token_id <= qwen_special_range_end);
            
            if is_special {
                let role_name = special_ids.get(&token_id).copied();
                
                // Check if this is a role marker (im_start) followed by a role
                if role_name == Some("im_start") && i + 1 < token_ids.len() {
                    // Save previous block if any
                    if !current_block_text.is_empty() {
                        blocks.push((current_role, current_block_text));
                        current_block_text = String::new();
                    }
                    
                    // Look ahead for role name
                    let next_token_id = token_ids[i+1];
                    if let Some(token) = self.reverse_vocab.get(&next_token_id) {
                        if token == "system" || token == "user" || token == "assistant" {
                            // Found a role, update current_role
                            current_role = Some(token);
                            // Found role marker
                            i += 2; // Skip both tokens
                            continue;
                        }
                    }
                    
                    current_role = None;
                    i += 1; // Skip just the im_start
                    continue;
                } 
                // If this is im_end, end current block
                else if role_name == Some("im_end") {
                    if !current_block_text.is_empty() {
                        blocks.push((current_role, current_block_text));
                        current_block_text = String::new();
                    }
                    current_role = None;
                    i += 1;
                    continue;
                }
                // For other special tokens, skip them
                else {
                    // Skip special token
                    i += 1;
                    continue;
                }
            }
            
            // For normal tokens, add to current block
            if let Some(token) = self.reverse_vocab.get(&token_id) {
                // For debugging: log tokens during decoding
                if i < 10 || i >= token_ids.len() - 5 {
                    // Process regular token
                }
                
                // Skip ChatML markers in normal text
                if token.starts_with("<|") && token.ends_with("|>") {
                    // These should have been handled as special tokens, but just in case
                    if token == "<|im_start|>" || token == "<|im_end|>" || 
                       token.contains("system") || token.contains("user") || token.contains("assistant") {
                        i += 1;
                        continue;
                    }
                }
                
                // Clean the token for decoding first
                let cleaned_token = self.clean_token_for_decoding(token);
                
                // Heuristic for subword detection:
                // 1. Don't add space before first token in a block
                // 2. Don't add space if token already has a leading space
                // 3. Don't add space if token is a short alphabetic sequence (likely a syllable/subword)
                let is_likely_subword = token.len() <= 3 && 
                                      token.chars().all(|c| c.is_ascii_alphabetic()) &&
                                      !token.starts_with('Ġ') && !token.starts_with('▁');
                                      
                let needs_space = self.should_add_space_before(token) && 
                                 !current_block_text.is_empty() && 
                                 !current_block_text.ends_with(' ') &&
                                 !cleaned_token.starts_with(' ') &&
                                 !is_likely_subword;
                                 
                // Only add space if truly needed
                if needs_space {
                    current_block_text.push(' ');
                }
                
                // Add the cleaned token
                current_block_text.push_str(&cleaned_token);
            } else {
                // If we can't find the token, add a placeholder
                // Unknown token
                current_block_text.push('�');
            }
            
            i += 1;
        }
        
        // Add any remaining block
        if !current_block_text.is_empty() {
            blocks.push((current_role, current_block_text));
        }
        
        // Now format all blocks with proper role markers
        for (role, text) in blocks {
            match role {
                Some("system") => {
                    result.push_str("\nSystem: ");
                    result.push_str(&text);
                    result.push('\n');
                },
                Some("user") => {
                    result.push_str("\nUser: ");
                    result.push_str(&text);
                    result.push('\n');
                },
                Some("assistant") => {
                    result.push_str("\nAssistant: ");
                    result.push_str(&text);
                    result.push('\n');
                },
                _ => {
                    // No role, just add the text
                    result.push_str(&text);
                }
            }
        }
        
        // Clean up any remaining ChatML markers
        let result = result.replace("<|im_start|>", "")
                          .replace("<|im_end|>", "")
                          .replace("<|endoftext|>", "")
                          .replace("<|system|>", "")
                          .replace("<|user|>", "")
                          .replace("<|assistant|>", "");
        
        // Remove excessive newlines and clean up whitespace
        let cleaned = result.lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        
        Ok(cleaned)
    }
    
    /// Clean a token for decoding, handling special whitespace markers
    /// Returns the cleaned token text with proper whitespace handling
    pub(crate) fn clean_token_for_decoding(&self, token: &str) -> String {
        // Handle GPT-2 style tokens (most common in Qwen3)
        if token.starts_with('Ġ') {
            // "Ġ" represents a space at the beginning of a token in GPT-2 style tokenizers
            let remaining = &token[token.chars().next().unwrap().len_utf8()..];
            return format!(" {}", remaining);
        }
        
        // Handle SentencePiece style tokens
        if token.starts_with('▁') {
            // "▁" represents a space at the beginning of a token in SentencePiece tokenizers
            let remaining = &token[token.chars().next().unwrap().len_utf8()..];
            return format!(" {}", remaining);
        }
        
        // Handle special character markers
        match token {
            "Ċ" => return "\n".to_string(),  // GPT-2 newline
            "Ĉ" => return "\t".to_string(),  // GPT-2 tab
            " " => return " ".to_string(),   // Explicit space token
            "\n" => return "\n".to_string(), // Explicit newline token
            "\t" => return "\t".to_string(), // Explicit tab token
            _ => {}
        }
        
        // Handle common subword continuation tokens (no space between these and previous token)
        if token.starts_with("##") {
            // BERT-style subword token
            return token.replace("##", "");
        }
        
        // Handle special cases of common punctuation and symbols
        let first_char = token.chars().next().unwrap_or(' ');
        if token.len() == 1 && first_char.is_ascii_punctuation() {
            // Single punctuation tokens should not have spaces added
            return token.to_string();
        }
        
        // Return the token unchanged if none of the above rules match
        token.to_string()
    }
    
    /// Check if we should add a space before a token during decoding
    /// Used for better tokenization of languages without explicit spaces
    pub(crate) fn should_add_space_before(&self, token: &str) -> bool {
        // Common GPT-2 or SentencePiece space markers are handled separately
        if token.starts_with('Ġ') || token.starts_with('▁') {
            return false; // These already encode spaces
        }
        
        // Special tokens with explicit space handling
        match token {
            // Special characters that shouldn't have spaces before them
            "," | "." | "!" | "?" | ":" | ";" | ")" | "]" | "}" | "\"" => false,
            
            // BPE continuation markers
            "##" => false,
            
            // Special whitespace tokens
            "Ċ" | "Ĉ" | " " | "\n" | "\t" => false,
            
            // Default behavior - let caller determine based on context
            _ => true
        }
    }
}