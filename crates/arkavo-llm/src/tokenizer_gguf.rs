use anyhow::Result;
use std::collections::HashMap;
use std::io::Cursor;
use candle_core::quantized::gguf_file;

pub struct GgufTokenizer {
    vocab: HashMap<String, u32>,
    reverse_vocab: HashMap<u32, String>,
    // Instead of Vec, use a merge map for O(1) lookup:
    merges: HashMap<(String, String), String>,
    special_tokens: HashMap<String, u32>,
    #[allow(dead_code)] // Currently unused but might be useful in future optimizations
    max_token_length: usize,
}

impl GgufTokenizer {
    pub fn vocab_size(&self) -> usize {
        self.vocab.len()
    }
    
    /// Tokenize a text string into tokens, implementing BPE encoding algorithm
    /// This handles the core BPE encoding without recursive calls
    fn tokenize_bytes(&self, text: &str) -> Vec<u32> {
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
    
    pub fn decode(&self, token_ids: &[u32]) -> Result<String> {
        println!("Decoding {} tokens", token_ids.len());
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
                            println!("Found role marker: {}", token);
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
                    println!("Skipping special token {}: {:?}", token_id, role_name);
                    i += 1;
                    continue;
                }
            }
            
            // For normal tokens, add to current block
            if let Some(token) = self.reverse_vocab.get(&token_id) {
                // For debugging: log tokens during decoding
                if i < 10 || i >= token_ids.len() - 5 {
                    println!("Token {}: ID {} = '{}'", i, token_id, token.replace("\n", "\\n"));
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
                println!("Unknown token ID: {}", token_id);
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
    fn clean_token_for_decoding(&self, token: &str) -> String {
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
        
        // Replace special token markers completely while preserving the token structure
        let cleaned = token
            .replace("##", "") // BERT-style continuation token
            .replace(['Ġ', '▁'], " ") // GPT-2 and SentencePiece space
            .replace('Ċ', "\n") // GPT-2 newline
            .replace('Ĉ', "\t"); // GPT-2 tab
        
        // Special rule for likely syllables or short word pieces
        if token.len() <= 3 && token.chars().all(|c| c.is_ascii_alphabetic()) {
            // Short alphabetic tokens are likely syllables that should join with surrounding tokens
            // without spaces, so we'll return them directly
            return cleaned;
        }
        
        cleaned
    }

    /// Determines whether a space should be added before a token
    /// This implements space-aware BPE decoding, which is common in LLM tokenizers
    fn should_add_space_before(&self, token: &str) -> bool {
        // Heuristics for determining if a token should have a space before it:
        
        // 1. Check for special space-indicating prefixes used by various tokenizers
        if token.starts_with("Ġ") || token.starts_with("▁") || token.starts_with("▃") {
            // These are special tokens that already encode a leading space
            return false;
        }
        
        // 2. If the token starts with a space, no need to add another
        if token.starts_with(' ') {
            return false;
        }
        
        // 3. Tokens starting with punctuation generally don't need spaces
        if let Some(first_char) = token.chars().next() {
            if first_char.is_ascii_punctuation() {
                return false;
            }
        }
        
        // 4. Special handling for common prefixes that indicate subword pieces
        // Most BPE tokenizers use prefixes like "##" to indicate word pieces
        if token.starts_with("##") {
            return false;
        }
        
        // 5. Check for numeric tokens
        if token.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        
        // 6. For any other token, add a space to ensure proper word boundaries
        // This is a conservative approach that may add extra spaces in some cases,
        // but it's better than running words together with no spaces
        true
    }
    
    pub fn new(model_bytes: &[u8]) -> Result<Self> {
        let mut gguf_data = Cursor::new(model_bytes);
        let gguf_content = gguf_file::Content::read(&mut gguf_data)?;

        // Count how many metadata keys we have
        println!("GGUF model has {} metadata keys", gguf_content.metadata.len());

        // Check specifically for Qwen3 vocab size info
        if let Some(gguf_file::Value::U32(vocab_size)) = gguf_content.metadata.get("qwen3.vocab_size") {
            println!("Found Qwen3 vocab_size: {}", vocab_size);
        }

        // Check for tokenizer.ggml special fields for Qwen3
        if let Some(gguf_file::Value::U32(eos_id)) = gguf_content.metadata.get("tokenizer.ggml.eos_token_id") {
            println!("Found EOS token ID: {}", eos_id);
        }

        if let Some(gguf_file::Value::U32(vocab_size)) = gguf_content.metadata.get("tokenizer.ggml.vocab_size") {
            println!("Found tokenizer vocab_size: {}", vocab_size);
        }

        if let Some(gguf_file::Value::String(tokenizer_type)) = gguf_content.metadata.get("tokenizer.ggml.type") {
            println!("Found tokenizer type: {}", tokenizer_type);
        }

        // Look for token-related keys specifically
        for key in gguf_content.metadata.keys() {
            if key.contains("token") || key.contains("vocab") {
                match gguf_content.metadata.get(key) {
                    Some(gguf_file::Value::Array(arr)) => {
                        println!("Found token array key: {} with {} elements", key, arr.len());
                        if !arr.is_empty() {
                            // Print the type of the first element
                            match &arr[0] {
                                gguf_file::Value::String(_) => println!("  - First element is a String"),
                                gguf_file::Value::Array(_) => println!("  - First element is an Array"),
                                _ => println!("  - First element is another type"),
                            }
                        }
                    },
                    Some(gguf_file::Value::U32(val)) => println!("Found token U32 key: {} = {}", key, val),
                    Some(gguf_file::Value::String(val)) => println!("Found token String key: {} = {}", key, val),
                    _ => {}
                }
            }
        }

        let mut vocab = HashMap::new();
        let mut reverse_vocab = HashMap::new();

        // Extract tokens from GGUF metadata - or create basic ASCII fallback if none
        Self::extract_tokens(&gguf_content, &mut vocab, &mut reverse_vocab);

        // Extract merges for BPE tokenizers
        let merges = Self::extract_merges(&gguf_content);

        // Extract special tokens and their IDs
        let special_tokens = Self::extract_special_tokens(&gguf_content);

        // Get max token length if available
        let max_token_length = if let Some(gguf_file::Value::U32(len)) =
            gguf_content.metadata.get("tokenizer.ggml.max_token_length") {
            *len as usize
        } else {
            128 // Default to 128 if not specified
        };

        println!("GGUF tokenizer initialized with {} tokens", vocab.len());

        // Debug: Print sample tokens from the vocabulary (beginning, middle, and end)
        println!("Sample of vocabulary tokens:");
        
        // First 50 tokens
        println!("First 50 vocabulary tokens:");
        for (token, &id) in vocab.iter().take(50) {
            println!("  Token ID {}: {:?} (len={})", id, token, token.len());
            
            // Check for whitespace or special characters
            if token.contains('Ġ') || token.contains('▁') || token.contains('Ċ') {
                println!("  Potential whitespace marker: {:?} => ID {}", token, id);
            }
            
            // Check for important tokens
            if token == " " || token == "\n" || token == "\t" || token.contains("<|") {
                println!("  Important token: {:?} => ID {}", token, id);
            }
            
            // Check for digit tokens to confirm encoding format
            if token.len() == 1 && token.chars().next().unwrap().is_ascii_digit() {
                println!("  Digit token: {:?} => ID {}", token, id);
            }
        }
        
        // Sample from middle of vocabulary
        println!("\nSample from middle of vocabulary (around ID 75000):");
        let mid_point = vocab.len() / 2;
        let mut mid_tokens = vocab.iter()
            .filter(|(_, &id)| id >= (mid_point as u32 - 5) && id <= (mid_point as u32 + 5))
            .collect::<Vec<_>>();
        mid_tokens.sort_by_key(|(_, &id)| id);
        for (token, &id) in mid_tokens {
            println!("  Token ID {}: {:?} (len={})", id, token, token.len());
        }
        
        // Last 50 tokens
        println!("\nLast 50 vocabulary tokens:");
        let mut last_tokens = vocab.iter()
            .filter(|(_, &id)| id >= (vocab.len() as u32 - 50))
            .collect::<Vec<_>>();
        last_tokens.sort_by_key(|(_, &id)| id);
        for (token, &id) in last_tokens {
            println!("  Token ID {}: {:?} (len={})", id, token, token.len());
            
            // Special attention to high ID tokens which are often special tokens
            if (151600..=151700).contains(&id) {
                println!("  High ID token in important range: {:?} => ID {}", token, id);
            }
        }
        
        // Specific search for whitespace markers
        println!("\nSearching for common whitespace markers:");
        let whitespace_markers = ["Ġ", "▁", "Ċ", "ĉ", " ", "\n", "\t"];
        for marker in &whitespace_markers {
            for (token, &id) in vocab.iter() {
                if token == marker {
                    println!("  Found exact whitespace marker: {:?} => ID {}", token, id);
                }
                // Also check for tokens starting with these markers as they often represent words with leading space
                else if token.starts_with(marker) && token.len() <= 5 {
                    // println!("  Found token starting with whitespace marker: {:?} => ID {}", token, id);
                }
            }
        }

        // Debug: Check if common tokens are in the vocabulary
        for &check in &[" ", "\n", "a", "t", "the"] {
            if let Some(&id) = vocab.get(check) {
                println!("  Common token {:?} found with ID {}", check, id);
            } else {
                println!("  WARNING: Common token {:?} NOT found in vocabulary", check);
            }
        }

        Ok(Self {
            vocab,
            reverse_vocab,
            merges,
            special_tokens,
            max_token_length,
        })
    }

    fn extract_tokens(
        gguf_content: &gguf_file::Content,
        vocab: &mut HashMap<String, u32>,
        reverse_vocab: &mut HashMap<u32, String>
    ) {
        // Try several common token vocab formats used in GGUF files

        // Format 1: Direct string array in tokenizer.ggml.tokens
        if let Some(gguf_file::Value::Array(tokens)) = gguf_content.metadata.get("tokenizer.ggml.tokens") {
            println!("Checking tokenizer.ggml.tokens array with {} elements", tokens.len());

            // Case 1a: Direct string array
            if tokens.iter().any(|v| matches!(v, gguf_file::Value::String(_))) {
                println!("Format: Direct string array");
                for (i, token_value) in tokens.iter().enumerate() {
                    if let gguf_file::Value::String(token) = token_value {
                        let token_id = i as u32;
                        vocab.insert(token.clone(), token_id);
                        reverse_vocab.insert(token_id, token.clone());
                    }
                }

                if !vocab.is_empty() {
                    println!("Loaded {} tokens from direct string array", vocab.len());

                    // Check for ChatML special tokens in the vocabulary
                    for i in 151640..151650 {
                        if i < tokens.len() {
                            if let gguf_file::Value::String(token) = &tokens[i] {
                                println!("High token ID {}: {:?}", i, token);
                            }
                        }
                    }

                    // We DO NOT manually add special tokens with arbitrary IDs!
                    // The GGUF model already has all the special tokens it needs
                    // Check for common whitespace and special tokens for logging purposes
                    let check_tokens = [
                        " ", "\n", "\t", ".", "!", "?", ",", ":", ";",
                        "<|im_start|>", "<|im_end|>", "<|endoftext|>"
                    ];

                    for &token in &check_tokens {
                        if let Some(&id) = vocab.get(token) {
                            println!("Found token {:?} in vocab with ID {}", token, id);
                        } else {
                            println!("WARNING: Token {:?} not found in vocab", token);
                        }
                    }

                    // Report token counts in high token ID ranges (for special tokens)
                    let special_ranges = [(151640, 151650), (150000, 150010), (100000, 100010)];
                    for (start, end) in special_ranges {
                        println!("Checking token ID range {}-{}:", start, end);
                        for id in start..end {
                            if let Some(token) = reverse_vocab.get(&id) {
                                println!("  ID {}: {:?}", id, token);
                            }
                        }
                    }

                    return;
                }
            }

            // Case 1b: Nested array - first element is an array of strings
            if let Some(gguf_file::Value::Array(token_strings)) = tokens.first() {
                println!("Format: Nested array of strings");
                for (i, token_value) in token_strings.iter().enumerate() {
                    if let gguf_file::Value::String(token) = token_value {
                        let token_id = i as u32;
                        vocab.insert(token.clone(), token_id);
                        reverse_vocab.insert(token_id, token.clone());
                    }
                }

                if !vocab.is_empty() {
                    println!("Loaded {} tokens from nested array", vocab.len());
                    return;
                }
            }
        }

        // Format 2: Check tokenizer.model.vocab (used in some llama.cpp GGUF files)
        if let Some(gguf_file::Value::Array(vocab_array)) = gguf_content.metadata.get("tokenizer.model.vocab") {
            println!("Checking tokenizer.model.vocab with {} elements", vocab_array.len());

            for (i, vocab_item) in vocab_array.iter().enumerate() {
                // Vocab items might be direct strings or arrays with token+score
                if let gguf_file::Value::String(token) = vocab_item {
                    let token_id = i as u32;
                    vocab.insert(token.clone(), token_id);
                    reverse_vocab.insert(token_id, token.clone());
                } else if let gguf_file::Value::Array(token_data) = vocab_item {
                    // First element is token, second might be score
                    if let Some(gguf_file::Value::String(token)) = token_data.first() {
                        let token_id = i as u32;
                        vocab.insert(token.clone(), token_id);
                        reverse_vocab.insert(token_id, token.clone());
                    }
                }
            }

            if !vocab.is_empty() {
                println!("Loaded {} tokens from tokenizer.model.vocab", vocab.len());
                return;
            }
        }

        // Format 3: Check 'vocab' key (used in some GGUF files)
        if let Some(gguf_file::Value::Array(vocab_array)) = gguf_content.metadata.get("vocab") {
            println!("Checking vocab with {} elements", vocab_array.len());

            for (i, token_value) in vocab_array.iter().enumerate() {
                if let gguf_file::Value::String(token) = token_value {
                    let token_id = i as u32;
                    vocab.insert(token.clone(), token_id);
                    reverse_vocab.insert(token_id, token.clone());
                }
            }

            if !vocab.is_empty() {
                println!("Loaded {} tokens from vocab key", vocab.len());
                return;
            }
        }

        // Try looking for a vocabulary size hint and token mappings
        let vocab_size = if let Some(gguf_file::Value::U32(size)) = gguf_content.metadata.get("tokenizer.ggml.vocab_size") {
            *size as usize
        } else if let Some(gguf_file::Value::U32(size)) = gguf_content.metadata.get("vocab_size") {
            *size as usize
        } else {
            0
        };

        if vocab_size > 0 {
            println!("Found vocab_size = {} in GGUF metadata", vocab_size);

            // Check if we have token ID mappings
            if let Some(gguf_file::Value::Array(token_id_pairs)) = gguf_content.metadata.get("tokenizer.ggml.token_id_pairs") {
                println!("Found token_id_pairs with {} entries", token_id_pairs.len());

                for token_pair in token_id_pairs {
                    if let gguf_file::Value::Array(pair) = token_pair {
                        if pair.len() >= 2 {
                            if let (Some(gguf_file::Value::String(token)), Some(gguf_file::Value::U32(id))) = (pair.first(), pair.get(1)) {
                                vocab.insert(token.clone(), *id);
                                reverse_vocab.insert(*id, token.clone());
                            }
                        }
                    }
                }

                if !vocab.is_empty() {
                    println!("Loaded {} tokens from token_id_pairs", vocab.len());
                    return;
                }
            }
        }

        // Fallback to basic ASCII vocabulary only if we couldn't find any tokens
        println!("WARNING: No tokens found in GGUF metadata, creating basic ASCII vocabulary");
        println!("This indicates a problem extracting the vocabulary from the GGUF file");
        println!("Text generation may produce incorrect results");

        // Generate a byte-level vocabulary - only as a last resort
        for i in 0..256 {
            let c = char::from_u32(i).unwrap_or('�');
            let token = c.to_string();
            vocab.insert(token.clone(), i);
            reverse_vocab.insert(i, token);
        }
    }

    fn extract_merges(gguf_content: &gguf_file::Content) -> HashMap<(String, String), String> {
        let mut merges = HashMap::new();

        // Debug: check what merge-related keys exist in the GGUF metadata
        for key in gguf_content.metadata.keys() {
            if key.contains("merge") || key.contains("bpe") {
                println!("Found potential merge data key: {}", key);
                match gguf_content.metadata.get(key) {
                    Some(gguf_file::Value::Array(arr)) => {
                        println!("  - Is an array with {} elements", arr.len());
                        if !arr.is_empty() {
                            match &arr[0] {
                                gguf_file::Value::Array(nested) => println!("  - First element is a nested array with {} elements", nested.len()),
                                gguf_file::Value::String(s) => println!("  - First element is a string: {}", s),
                                _ => println!("  - First element is some other type")
                            }
                        }
                    },
                    Some(_) => println!("  - Not an array type"),
                    None => {}
                }
            }
        }

        // Process merges from tokenizer.ggml.merges (main format)
        if let Some(gguf_file::Value::Array(merge_values)) = gguf_content.metadata.get("tokenizer.ggml.merges") {
            println!("Found tokenizer.ggml.merges with {} entries", merge_values.len());

            // Sample first few merges for debugging
            let sample_size = 5.min(merge_values.len());
            println!("First {} merge entries (sample):", sample_size);
            for (idx, merge_value) in merge_values.iter().take(sample_size).enumerate() {
                if let gguf_file::Value::String(merge) = merge_value {
                    println!("  [{}]: {}", idx, merge);
                }
            }

            // Process all merges directly from the array - DO NOT look for nested array
            let mut byte_token_merges = 0;
            let mut success_count = 0;
            let mut failed_parse = 0;

            for (idx, merge_value) in merge_values.iter().enumerate() {
                if let gguf_file::Value::String(merge) = merge_value {
                    // Parse merge entry (format is typically "first second" or "first second result")
                    let parts: Vec<&str> = merge.split_whitespace().collect();
                    if parts.len() >= 2 {
                        // Check if this is a byte-level merge
                        let first = parts[0].to_string();
                        let second = parts[1].to_string();

                        // See if these look like byte tokens
                        if first.parse::<u8>().is_ok() || second.parse::<u8>().is_ok() {
                            byte_token_merges += 1;
                        }

                        let result = if parts.len() > 2 {
                            parts[2].to_string()
                        } else {
                            format!("{}{}", first, second)
                        };

                        merges.insert((first, second), result);
                        success_count += 1;

                        // Print progress for large merge sets
                        if idx > 0 && idx % 10000 == 0 {
                            println!("Processed {} merges...", idx);
                        }
                    } else {
                        failed_parse += 1;
                        if failed_parse <= 5 {
                            println!("WARNING: Failed to parse merge entry: {}", merge);
                        }
                    }
                }
            }

            println!("Successfully loaded {} merge pairs into HashMap", success_count);
            println!("Found {} byte-token merges", byte_token_merges);

            if failed_parse > 0 {
                println!("WARNING: Failed to parse {} merge entries", failed_parse);
            }

            // For backwards compatibility, also check if the first element is a nested array
            // (some older GGUF models might use this format)
            if success_count == 0 && !merge_values.is_empty() {
                if let Some(gguf_file::Value::Array(merge_pairs)) = merge_values.first() {
                    println!("Found legacy format: nested array with {} entries", merge_pairs.len());

                    // Process all merges in the nested array
                    for merge_value in merge_pairs.iter() {
                        if let gguf_file::Value::String(merge) = merge_value {
                            // Parse merge entry (format is typically "first second result")
                            let parts: Vec<&str> = merge.split_whitespace().collect();
                            if parts.len() >= 2 {
                                let first = parts[0].to_string();
                                let second = parts[1].to_string();

                                let result = if parts.len() > 2 {
                                    parts[2].to_string()
                                } else {
                                    format!("{}{}", first, second)
                                };

                                merges.insert((first, second), result);
                                success_count += 1;
                            }
                        }
                    }

                    println!("Legacy format: loaded {} merge pairs", success_count);
                }
            }

            // Check for important merges - space, newline, etc.
            let critical_bytes = [32, 10, 9]; // space, newline, tab
            for &byte in &critical_bytes {
                let byte_str = format!("{}", byte);
                let char_str = char::from_u32(byte as u32).unwrap_or('�').to_string();
                let mut found = false;

                // Check both formats - byte value as string and character representation
                for ((first, second), result) in &merges {
                    if first == &byte_str || second == &byte_str || result == &byte_str ||
                        first == &char_str || second == &char_str || result == &char_str {
                        found = true;
                        println!("Critical byte {} ({:?}) found in merges: {} + {} -> {}",
                                 byte, char::from_u32(byte as u32).unwrap_or('�'), first, second, result);
                    }
                }

                if !found {
                    println!("WARNING: Critical byte {} ({:?}) NOT found in any merges",
                             byte, char::from_u32(byte as u32).unwrap_or('�'));
                }
            }
        } else {
            println!("WARNING: No 'tokenizer.ggml.merges' found in GGUF metadata");

            // Try alternative merge keys if the standard one isn't found
            if let Some(gguf_file::Value::Array(bpe_merges)) = gguf_content.metadata.get("tokenizer.model.merges") {
                println!("Found alternative 'tokenizer.model.merges' with {} entries", bpe_merges.len());

                // Process all merges in the alternative format
                let mut success_count = 0;
                for merge_value in bpe_merges.iter() {
                    if let gguf_file::Value::String(merge) = merge_value {
                        // Parse merge entry
                        let parts: Vec<&str> = merge.split_whitespace().collect();
                        if parts.len() >= 2 {
                            let first = parts[0].to_string();
                            let second = parts[1].to_string();

                            let result = if parts.len() > 2 {
                                parts[2].to_string()
                            } else {
                                format!("{}{}", first, second)
                            };

                            merges.insert((first, second), result);
                            success_count += 1;
                        }
                    }
                }

                println!("Successfully loaded {} merge pairs from alternative format", success_count);
            }
        }

        // If no merges found, print a warning
        if merges.is_empty() {
            println!("WARNING: No merge rules loaded! Tokenization will likely produce many <unk> tokens");
        } else {
            println!("Total merges loaded: {}", merges.len());

            // Sample a few merges to show what they look like
            println!("Sample of loaded merges:");
            let mut count = 0;
            for ((first, second), result) in &merges {
                if count < 10 {
                    println!("  {} + {} -> {}", first, second, result);
                    count += 1;
                } else {
                    break;
                }
            }
        }

        merges
    }

    fn extract_special_tokens(gguf_content: &gguf_file::Content) -> HashMap<String, u32> {
        let mut special_tokens = HashMap::new();

        // Check GGUF metadata for explicit special token IDs
        let special_token_keys = [
            "tokenizer.ggml.bos_token_id",
            "tokenizer.ggml.eos_token_id",
            "tokenizer.ggml.unk_token_id",
            "tokenizer.ggml.sep_token_id",
            "tokenizer.ggml.pad_token_id",
        ];

        for key in special_token_keys {
            if let Some(gguf_file::Value::U32(token_id)) = gguf_content.metadata.get(key) {
                let token_name = key.split('.').next_back()
                    .unwrap_or(key)
                    .replace("_token_id", "");

                println!("Found special token in metadata: {} = {}", token_name, token_id);
                special_tokens.insert(token_name, *token_id);
            }
        }

        // For Qwen3, check for special tokens in the high token ID range
        // These are typically role markers like <|im_start|>, <|im_end|>, etc.

        // First, look for known special token mappings in GGUF metadata
        if let Some(gguf_file::Value::Array(token_mappings)) = gguf_content.metadata.get("tokenizer.chat_template.special_tokens") {
            println!("Found chat template special tokens: {} entries", token_mappings.len());

            for mapping in token_mappings {
                if let gguf_file::Value::Array(pair) = mapping {
                    if pair.len() >= 2 {
                        if let (Some(gguf_file::Value::String(name)), Some(gguf_file::Value::U32(id))) = (pair.first(), pair.get(1)) {
                            // Remove angle brackets if present
                            let clean_name = name.trim_start_matches('<').trim_end_matches('>');
                            println!("Chat template special token: {} -> ID {}", name, id);
                            special_tokens.insert(clean_name.to_string(), *id);

                            // Also add common variations
                            if name.contains("im_start") {
                                special_tokens.insert("im_start".to_string(), *id);
                            } else if name.contains("im_end") {
                                special_tokens.insert("im_end".to_string(), *id);
                            }
                        }
                    }
                }
            }
        }

        // Get vocab size
        let vocab_size = if let Some(gguf_file::Value::U32(size)) = gguf_content.metadata.get("tokenizer.ggml.vocab_size") {
            *size as usize
        } else if let Some(gguf_file::Value::U32(size)) = gguf_content.metadata.get("vocab_size") {
            *size as usize
        } else {
            151936 // Qwen3 default
        };

        // Qwen3-specific special tokens - high token IDs
        println!("Checking for Qwen3-specific special tokens in high ID range");

        // Define token strings to look for with UNIQUE IDs for each token
        let qwen_tokens = [
            ("<|im_start|>", 151643),     // Start marker
            ("<|im_end|>", 151645),       // End marker
            ("<|system|>", 151646),       // System role - unique ID
            ("<|user|>", 151647),         // User role - unique ID
            ("<|assistant|>", 151648),    // Assistant role - unique ID
            ("<|endoftext|>", 151649),    // End of text marker
        ];

        // First check token string arrays for these special tokens
        if let Some(gguf_file::Value::Array(tokens)) = gguf_content.metadata.get("tokenizer.ggml.tokens") {
            // Try to locate the exact special tokens first
            for &(token_str, id_hint) in &qwen_tokens {
                let mut found = false;

                // First check the hint ID directly
                if let Some(gguf_file::Value::String(s)) = tokens.get(id_hint) {
                    if s.contains(token_str) || token_str.contains(s) {
                        let name = token_str.trim_start_matches('<').trim_start_matches('|')
                            .trim_end_matches('>').trim_end_matches('|');
                        println!("Found special token at hint ID {}: {} = {}", id_hint, name, token_str);
                        special_tokens.insert(name.to_string(), id_hint as u32);
                        found = true;
                    }
                }

                // If not found at hint, search in a range around high token IDs
                if !found {
                    // For Qwen3, search in the high token ID range (151640-151650)
                    for i in 151640..151650 {
                        if i < tokens.len() {
                            if let gguf_file::Value::String(s) = &tokens[i] {
                                if s.contains(token_str) || token_str.contains(s) {
                                    let name = token_str.trim_start_matches('<').trim_start_matches('|')
                                        .trim_end_matches('>').trim_end_matches('|');
                                    println!("Found special token at ID {}: {} = {}", i, name, s);
                                    special_tokens.insert(name.to_string(), i as u32);
                                    found = true;
                                    break;
                                }
                            }
                        }
                    }

                    // If still not found, search the whole vocabulary
                    if !found {
                        for (i, val) in tokens.iter().enumerate() {
                            if i < vocab_size {
                                if let gguf_file::Value::String(s) = val {
                                    if s == token_str {
                                        let name = token_str.trim_start_matches('<').trim_start_matches('|')
                                            .trim_end_matches('>').trim_end_matches('|');
                                        println!("Found special token in full vocab scan: {} = {} (ID {})", name, s, i);
                                        special_tokens.insert(name.to_string(), i as u32);
                                        found = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }

                // If we couldn't find it anywhere, use the hint ID as a fallback
                if !found {
                    let name = token_str.trim_start_matches('<').trim_start_matches('|')
                        .trim_end_matches('>').trim_end_matches('|');
                    println!("Using fallback ID for special token {}: ID {}", name, id_hint);
                    special_tokens.insert(name.to_string(), id_hint as u32);
                }
            }
        }

        // Special handling for Qwen-specific tokens
        // ChatML tokens for Qwen3 - set defaults if not found above
        if !special_tokens.contains_key("im_start") {
            special_tokens.insert("im_start".to_string(), 151643);
            println!("Using fallback ID for <|im_start|>: 151643");
        }

        if !special_tokens.contains_key("im_end") {
            special_tokens.insert("im_end".to_string(), 151645);
            println!("Using fallback ID for <|im_end|>: 151645");
        }

        // Set standard special tokens if not already set
        if !special_tokens.contains_key("bos") {
            special_tokens.insert("bos".to_string(), *special_tokens.get("im_start").unwrap_or(&1));
        }

        if !special_tokens.contains_key("eos") {
            special_tokens.insert("eos".to_string(), *special_tokens.get("im_end").unwrap_or(&2));
        }

        if !special_tokens.contains_key("unk") {
            special_tokens.insert("unk".to_string(), 0);
        }

        // Add full special token strings
        let mut tokens_with_brackets = HashMap::new();
        for (name, &id) in &special_tokens {
            if name == "im_start" {
                tokens_with_brackets.insert("<|im_start|>".to_string(), id);
            } else if name == "im_end" {
                tokens_with_brackets.insert("<|im_end|>".to_string(), id);
            } else if name == "system" {
                tokens_with_brackets.insert("<|system|>".to_string(), id);
            } else if name == "user" {
                tokens_with_brackets.insert("<|user|>".to_string(), id);
            } else if name == "assistant" {
                tokens_with_brackets.insert("<|assistant|>".to_string(), id);
            }
        }

        // Merge the bracketed versions back
        for (token, id) in tokens_with_brackets {
            special_tokens.insert(token, id);
        }

        special_tokens
    }
}