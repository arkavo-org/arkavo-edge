use anyhow::Result;
use std::collections::HashMap;
use std::io::Cursor;
use candle_core::quantized::gguf_file;
use crate::tokenizer_gguf_core::GgufTokenizer;

impl GgufTokenizer {
    /// Create a new tokenizer from GGUF model bytes
    pub fn new(model_bytes: &[u8]) -> Result<Self> {
        let mut gguf_data = Cursor::new(model_bytes);
        let gguf_content = gguf_file::Content::read(&mut gguf_data)?;

        // Track metadata without excessive logging

        // Scan for token-related metadata

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

        // Check if we have a valid vocabulary size
        if vocab.len() < 1000 {
            println!("WARNING: Small vocabulary size ({}). This may indicate tokenizer initialization issues.", vocab.len());
        }

        Ok(Self {
            vocab,
            reverse_vocab,
            merges,
            special_tokens,
            max_token_length,
        })
    }

    /// Extract tokens from GGUF metadata
    fn extract_tokens(
        gguf_content: &gguf_file::Content,
        vocab: &mut HashMap<String, u32>,
        reverse_vocab: &mut HashMap<u32, String>
    ) {
        // Try several common token vocab formats used in GGUF files

        // Format 1: Direct string array in tokenizer.ggml.tokens
        if let Some(gguf_file::Value::Array(tokens)) = gguf_content.metadata.get("tokenizer.ggml.tokens") {
            // Case 1a: Direct string array
            if tokens.iter().any(|v| matches!(v, gguf_file::Value::String(_))) {
                for (i, token_value) in tokens.iter().enumerate() {
                    if let gguf_file::Value::String(token) = token_value {
                        let token_id = i as u32;
                        vocab.insert(token.clone(), token_id);
                        reverse_vocab.insert(token_id, token.clone());
                    }
                }

                if !vocab.is_empty() {

                    return;
                }
            }

            // Case 1b: Nested array - first element is an array of strings
            if let Some(gguf_file::Value::Array(token_strings)) = tokens.first() {
                for (i, token_value) in token_strings.iter().enumerate() {
                    if let gguf_file::Value::String(token) = token_value {
                        let token_id = i as u32;
                        vocab.insert(token.clone(), token_id);
                        reverse_vocab.insert(token_id, token.clone());
                    }
                }

                if !vocab.is_empty() {
                    return;
                }
            }
        }

        // Format 2: Check tokenizer.model.vocab (used in some llama.cpp GGUF files)
        if let Some(gguf_file::Value::Array(vocab_array)) = gguf_content.metadata.get("tokenizer.model.vocab") {
            // Check tokenizer.model.vocab format

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
                return;
            }
        }

        // Format 3: Check 'vocab' key (used in some GGUF files)
        if let Some(gguf_file::Value::Array(vocab_array)) = gguf_content.metadata.get("vocab") {
            // Check vocab format

            for (i, token_value) in vocab_array.iter().enumerate() {
                if let gguf_file::Value::String(token) = token_value {
                    let token_id = i as u32;
                    vocab.insert(token.clone(), token_id);
                    reverse_vocab.insert(token_id, token.clone());
                }
            }

            if !vocab.is_empty() {
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

    /// Extract BPE merges from GGUF metadata
    fn extract_merges(gguf_content: &gguf_file::Content) -> HashMap<(String, String), String> {
        let mut merges = HashMap::new();

        // Debug: check what merge-related keys exist in the GGUF metadata
        for key in gguf_content.metadata.keys() {
            if key.contains("merge") || key.contains("bpe") {
                match gguf_content.metadata.get(key) {
                    Some(gguf_file::Value::Array(_arr)) => {
                        // Check array contents
                    },
                    Some(_) => { /* Not an array */ },
                    None => {}
                }
            }
        }

        // Process merges from tokenizer.ggml.merges (main format)
        if let Some(gguf_file::Value::Array(merge_values)) = gguf_content.metadata.get("tokenizer.ggml.merges") {
            // Process merges from array

            // Process all merges directly from the array - DO NOT look for nested array
            let mut _byte_token_merges = 0;
            let success_count = 0;
            let mut _failed_parse = 0;

            for (_idx, merge_value) in merge_values.iter().enumerate() {
                if let gguf_file::Value::String(merge) = merge_value {
                    // Parse merge entry (format is typically "first second" or "first second result")
                    let parts: Vec<&str> = merge.split_whitespace().collect();
                    if parts.len() >= 2 {
                        // Check if this is a byte-level merge
                        let first = parts[0].to_string();
                        let second = parts[1].to_string();

                        // See if these look like byte tokens
                        if first.parse::<u8>().is_ok() || second.parse::<u8>().is_ok() {
                            _byte_token_merges += 1;
                        }

                        let result = if parts.len() > 2 {
                            parts[2].to_string()
                        } else {
                            format!("{}{}", first, second)
                        };

                        merges.insert((first, second), result);

                        // Process large merge sets efficiently
                    } else {
                        _failed_parse += 1;
                        if _failed_parse <= 5 {
                            // Failed to parse merge entry
                        }
                    }
                }
            }

            // Loaded merge pairs successfully

            // For backwards compatibility, also check if the first element is a nested array
            // (some older GGUF models might use this format)
            if success_count == 0 && !merge_values.is_empty() {
                if let Some(gguf_file::Value::Array(merge_pairs)) = merge_values.first() {
                    // Found legacy format nested array

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
                            }
                        }
                    }

                    // Legacy format loaded
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
                        // Critical byte found in merges
                    }
                }

                if !found {
                    // Critical byte not found
                }
            }
        } else {
            // No standard merges found in metadata

            // Try alternative merge keys if the standard one isn't found
            if let Some(gguf_file::Value::Array(bpe_merges)) = gguf_content.metadata.get("tokenizer.model.merges") {
                // Found alternative merges

                // Process all merges in the alternative format
                let mut _success_count = 0;
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
                        }
                    }
                }

                // Loaded merge pairs from alternative format
            }
        }

        // If no merges found, print a warning
        if merges.is_empty() {
            // No merge rules found (warning)
        } else {
            // Merges loaded successfully
        }

        merges
    }

    /// Extract special tokens from GGUF metadata
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

                // Found special token in metadata
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
        // Check for Qwen3-specific special tokens in high ID range

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
                        // Found special token at hint ID
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
                                    // Found special token at ID
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
                                        // Found special token in full vocab scan
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
                    // Using fallback ID
                    special_tokens.insert(name.to_string(), id_hint as u32);
                }
            }
        }

        // Special handling for Qwen-specific tokens
        // ChatML tokens for Qwen3 - set defaults if not found above
        if !special_tokens.contains_key("im_start") {
            special_tokens.insert("im_start".to_string(), 151643);
            // Using fallback ID for im_start
        }

        if !special_tokens.contains_key("im_end") {
            special_tokens.insert("im_end".to_string(), 151645);
            // Using fallback ID for im_end
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