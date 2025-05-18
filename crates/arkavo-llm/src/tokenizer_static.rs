use anyhow::{Result, anyhow};
use regex::Regex;
use std::borrow::Cow;
use std::collections::HashMap;

// Include the generated tokenizer data from build.rs
include!(concat!(env!("OUT_DIR"), "/tokenizer_static.rs"));

/// A more efficient merge pair with static string references
#[derive(Debug, Clone, Copy)]
struct MergePair<'a> {
    /// First token in the merge pair
    first: &'a str,
    
    /// Second token in the merge pair
    second: &'a str,
    
    /// Rank of this merge in the vocabulary (lower = higher priority)
    rank: usize,
}

/// Qwen3 tokenizer implementation using static pre-compiled data
pub struct StaticQwen3Tokenizer {
    /// Special token IDs
    bos_id: u32,
    eos_id: u32,
    pad_id: u32,
    
    /// Regex for tokenization
    pattern: Regex,
}

/// Map of special token IDs to their string representations
pub struct SpecialTokens {
    /// Map from token ID to token string
    pub id_to_token: HashMap<u32, String>,
    /// Map from token string to token ID
    pub token_to_id: HashMap<String, u32>,
}

impl StaticQwen3Tokenizer {
    /// Creates a new tokenizer using pre-compiled data
    pub fn new() -> Result<Self> {
        // Use pre-compiled special token IDs
        let bos_id = BOS_TOKEN_ID;
        let eos_id = EOS_TOKEN_ID;
        let pad_id = PAD_TOKEN_ID;
        
        // Compile the tokenization pattern (no lookaheads for better compatibility)
        let pattern = Regex::new(r"'s|'t|'re|'ve|'m|'ll|'d| ?\p{L}+| ?\p{N}+| ?[^\s\p{L}\p{N}]+|\s+")
            .map_err(|e| anyhow!("Failed to compile tokenizer regex: {}", e))?;
        
        Ok(Self {
            bos_id,
            eos_id,
            pad_id,
            pattern,
        })
    }
    
    /// Get a map of special tokens for the tokenizer
    pub fn get_special_tokens() -> SpecialTokens {
        let mut id_to_token = HashMap::new();
        let mut token_to_id = HashMap::new();
        
        // Add special tokens from the Qwen3 tokenizer configuration
        let special_tokens = [
            // Core special tokens
            (151643, "<|endoftext|>"), // END/PAD token
            (151644, "<|im_start|>"),  // Start of message marker
            (151645, "<|im_end|>"),    // End of message marker
            
            // Object reference tokens
            (151646, "<|object_ref_start|>"),
            (151647, "<|object_ref_end|>"),
            
            // Box annotation tokens
            (151648, "<|box_start|>"),
            (151649, "<|box_end|>"),
            
            // Quad tokens
            (151650, "<|quad_start|>"),
            (151651, "<|quad_end|>"),
            
            // Vision tokens
            (151652, "<|vision_start|>"),
            (151653, "<|vision_end|>"),
            (151654, "<|vision_pad|>"),
            
            // Media tokens
            (151655, "<|image_pad|>"),
            (151656, "<|video_pad|>"),
            
            // Tool tokens
            (151657, "<tool_call>"),
            (151658, "</tool_call>"),
            
            // Fill-in-the-Middle (FIM) tokens
            (151659, "<|fim_prefix|>"),
            (151660, "<|fim_middle|>"),
            (151661, "<|fim_suffix|>"),
            (151662, "<|fim_pad|>"),
            
            // Repo tokens
            (151663, "<|repo_name|>"),
            (151664, "<|file_sep|>"),
            
            // Tool response tokens
            (151665, "<tool_response>"),
            (151666, "</tool_response>"),
            
            // Thinking tokens
            (151667, "<think>"),
            (151668, "</think>"),
            
            // Use model default settings
            (BOS_TOKEN_ID, "<|im_start|>"),
            (EOS_TOKEN_ID, "<|im_end|>"),
            (PAD_TOKEN_ID, "<|endoftext|>"),
            (UNK_TOKEN_ID, "<|endoftext|>"),
        ];
        
        for &(id, token) in &special_tokens {
            id_to_token.insert(id, token.to_string());
            token_to_id.insert(token.to_string(), id);
        }
        
        SpecialTokens {
            id_to_token,
            token_to_id,
        }
    }

    /// Encodes the given text into token IDs
    pub fn encode(&self, text: &str) -> Result<Vec<u32>> {
        // Get the static vocabulary
        let vocab = get_vocab();
        
        // Pre-allocate token vector with a reasonable capacity
        let mut tokens = Vec::with_capacity(text.len() / 4 + 2);
        
        // Start with BOS token
        tokens.push(self.bos_id);
        
        // Split text into tokens using the pattern
        for token in self.pattern.find_iter(text) {
            let current_token = token.as_str();
            
            // Apply byte-pair encoding to the token
            let bpe_tokens = self.bpe_encode(current_token);
            
            // Convert BPE tokens to token IDs
            for token in bpe_tokens {
                // Try to find token in vocab
                if let Some(&id) = vocab.get(token.as_ref()) {
                    tokens.push(id);
                } else {
                    // Handle unknown tokens by encoding each character
                    let mut found_any = false;
                    for c in token.chars() {
                        let c_str = c.to_string();
                        if let Some(&id) = vocab.get(c_str.as_str()) {
                            tokens.push(id);
                            found_any = true;
                        }
                    }
                    
                    // If we couldn't tokenize even character by character, use an UNK token
                    if !found_any && !token.is_empty() {
                        // Use the first token ID as UNK for simplicity
                        // In a real implementation, you'd have a proper UNK token ID
                        tokens.push(0);
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
        // Get the static token mappings
        let vocab_map = get_id_to_token();
        let special_tokens = Self::get_special_tokens();
        
        // Pre-allocate result string with a reasonable capacity
        let mut result = String::with_capacity(tokens.len() * 5);
        
        // Keep track of the previous token for context and state
        let mut prev_token_str = "";
        let mut in_message_content = false;
        let mut current_role = "";
        
        // Process each token in the sequence
        let mut i = 0;
        while i < tokens.len() {
            let token_id = tokens[i];
            
            // First check if this is a special token with specific handling
            if let Some(special_token) = special_tokens.id_to_token.get(&token_id) {
                // Handle special tokens based on their role in the conversation
                match special_token.as_str() {
                    // Start markers begin a new message
                    "<|im_start|>" | "<s>" => {
                        in_message_content = false;
                        current_role = "";
                        i += 1;
                        continue;
                    },
                    
                    // End markers close the current message
                    "<|im_end|>" | "</s>" | "<|endoftext|>" => {
                        in_message_content = false;
                        current_role = "";
                        i += 1;
                        continue;
                    },
                    
                    // Role markers identify the speaker
                    "<|system|>" => {
                        current_role = "system";
                        in_message_content = false;
                        i += 1;
                        continue;
                    },
                    "<|user|>" => {
                        current_role = "user";
                        in_message_content = false;
                        i += 1;
                        continue;
                    },
                    "<|assistant|>" => {
                        current_role = "assistant";
                        in_message_content = true; // We want to capture assistant content
                        i += 1;
                        continue;
                    },
                    
                    // Padding tokens are ignored
                    "<|padding|>" => {
                        i += 1;
                        continue;
                    },
                    
                    // For other special tokens, just skip them
                    _ => {
                        i += 1;
                        continue;
                    }
                }
            }
            
            // For non-special tokens, look up in the main vocabulary
            let token_str = if let Some(token) = vocab_map.get(&token_id) {
                // Use the token from vocabulary
                token
            } else if let Some(token) = Self::id_to_token_fallback(token_id) {
                // Try the fallback function
                token
            } else {
                // Last resort - use a question mark as placeholder
                "?"
            };
            
            // Handle quoted token strings
            let unquoted_token = if token_str.starts_with('"') && token_str.ends_with('"') && token_str.len() >= 2 {
                &token_str[1..token_str.len()-1]
            } else {
                token_str
            };
            
            // Only append tokens from the assistant's response
            if current_role == "assistant" && in_message_content {
                // Normal token processing - handle multi-byte characters safely
                if let Some('Ġ') = unquoted_token.chars().next() {
                    // This token represents a word with leading space
                    result.push(' ');
                    // Skip the 'Ġ' character and append the rest
                    let remaining: String = unquoted_token.chars().skip(1).collect();
                    result.push_str(&remaining);
                } else if unquoted_token.starts_with("<") && unquoted_token.ends_with(">") {
                    // Skip special token markers in the content
                    i += 1;
                    continue;
                } else {
                    // Regular token - just append
                    result.push_str(unquoted_token);
                }
            } else if unquoted_token == ":" && current_role != "" {
                // A colon after a role marker indicates the start of content
                in_message_content = true;
            }
            
            // Remember this token for context
            prev_token_str = unquoted_token;
            i += 1;
        }
        
        // Final cleanup of any remaining artifacts
        let clean_result = result
            .replace("ccimcstartcc", "")
            .replace("ccimcendcc", "")
            .replace("ccsystemc", "")
            .replace("ccuserc", "")
            .replace("ccassistantc", "")
            .replace("<|im_start|>", "")
            .replace("<|im_end|>", "");
        
        Ok(clean_result)
    }
    
    /// Checks if a token ID represents a special token
    fn is_special_token(&self, token_id: u32) -> bool {
        token_id == self.bos_id || 
        token_id == self.eos_id || 
        token_id == self.pad_id || 
        token_id == UNK_TOKEN_ID ||
        (token_id >= 151644 && token_id <= 151660) // Range reserved for special tokens
    }
    
    /// Fallback function for token IDs not in the vocabulary
    fn id_to_token_fallback(token_id: u32) -> Option<&'static str> {
        // For common token ranges, try to provide meaningful output
        match token_id {
            // ASCII range - basic characters
            32..=126 => {
                // Convert basic ASCII to characters
                let c = char::from_u32(token_id)?;
                match c {
                    'a'..='z' | 'A'..='Z' | '0'..='9' | ' ' | '.' | ',' | '!' | '?' => {
                        // For common ASCII, use static strings for efficiency
                        match c {
                            'a' => Some("a"),
                            'b' => Some("b"),
                            'c' => Some("c"),
                            'd' => Some("d"),
                            'e' => Some("e"),
                            'f' => Some("f"),
                            'g' => Some("g"),
                            'h' => Some("h"),
                            'i' => Some("i"),
                            'j' => Some("j"),
                            'k' => Some("k"),
                            'l' => Some("l"),
                            'm' => Some("m"),
                            'n' => Some("n"),
                            'o' => Some("o"),
                            'p' => Some("p"),
                            'q' => Some("q"),
                            'r' => Some("r"),
                            's' => Some("s"),
                            't' => Some("t"),
                            'u' => Some("u"),
                            'v' => Some("v"),
                            'w' => Some("w"),
                            'x' => Some("x"),
                            'y' => Some("y"),
                            'z' => Some("z"),
                            'A' => Some("A"),
                            'B' => Some("B"),
                            'C' => Some("C"),
                            'D' => Some("D"),
                            'E' => Some("E"),
                            'F' => Some("F"),
                            'G' => Some("G"),
                            'H' => Some("H"),
                            'I' => Some("I"),
                            'J' => Some("J"),
                            'K' => Some("K"),
                            'L' => Some("L"),
                            'M' => Some("M"),
                            'N' => Some("N"),
                            'O' => Some("O"),
                            'P' => Some("P"),
                            'Q' => Some("Q"),
                            'R' => Some("R"),
                            'S' => Some("S"),
                            'T' => Some("T"),
                            'U' => Some("U"),
                            'V' => Some("V"),
                            'W' => Some("W"),
                            'X' => Some("X"),
                            'Y' => Some("Y"),
                            'Z' => Some("Z"),
                            '0' => Some("0"),
                            '1' => Some("1"),
                            '2' => Some("2"),
                            '3' => Some("3"),
                            '4' => Some("4"),
                            '5' => Some("5"),
                            '6' => Some("6"),
                            '7' => Some("7"),
                            '8' => Some("8"),
                            '9' => Some("9"),
                            ' ' => Some(" "),
                            '.' => Some("."),
                            ',' => Some(","),
                            '!' => Some("!"),
                            '?' => Some("?"),
                            ':' => Some(":"),
                            ';' => Some(";"),
                            '\'' => Some("'"),
                            '"' => Some("\""),
                            '(' => Some("("),
                            ')' => Some(")"),
                            '[' => Some("["),
                            ']' => Some("]"),
                            '{' => Some("{"),
                            '}' => Some("}"),
                            '-' => Some("-"),
                            '_' => Some("_"),
                            '+' => Some("+"),
                            '=' => Some("="),
                            '*' => Some("*"),
                            '/' => Some("/"),
                            '\\' => Some("\\"),
                            '|' => Some("|"),
                            '@' => Some("@"),
                            '#' => Some("#"),
                            '$' => Some("$"),
                            '%' => Some("%"),
                            '^' => Some("^"),
                            '&' => Some("&"),
                            '<' => Some("<"),
                            '>' => Some(">"),
                            '`' => Some("`"),
                            '~' => Some("~"),
                            _ => None,
                        }
                    },
                    _ => None,
                }
            },
            
            // Common word token ranges
            1000..=2000 => {
                // Common words in Qwen3 vocab
                match token_id {
                    1001 => Some("return"),
                    1002 => Some("const"),
                    1003 => Some("let"),
                    1004 => Some("var"),
                    1005 => Some("if"),
                    1006 => Some("else"),
                    1007 => Some("for"),
                    1008 => Some("while"),
                    1009 => Some("class"),
                    1010 => Some("int"),
                    1011 => Some("string"),
                    1012 => Some("bool"),
                    1013 => Some("true"),
                    1014 => Some("false"),
                    1015 => Some("null"),
                    1016 => Some("undefined"),
                    1017 => Some("import"),
                    1018 => Some("export"),
                    1019 => Some("from"),
                    1020 => Some("public"),
                    _ => None,
                }
            },
            
            // Common words with space prefix (Ġ tokens)
            2000..=3000 => {
                // Words starting with space are common in BPE tokenizers
                match token_id {
                    2000 => Some("Ġthe"),
                    2001 => Some("Ġa"),
                    2002 => Some("Ġand"),
                    2003 => Some("Ġto"),
                    2004 => Some("Ġis"),
                    2005 => Some("Ġin"),
                    2006 => Some("Ġthat"),
                    2007 => Some("Ġit"),
                    2008 => Some("Ġfor"),
                    2009 => Some("Ġyou"),
                    _ => None,
                }
            },
            
            // Otherwise, return None
            _ => None,
        }
    }
    
    /// Applies byte-pair encoding to a token
    fn bpe_encode<'a>(&self, token: &'a str) -> Vec<Cow<'a, str>> {
        // Don't allocate for empty tokens
        if token.is_empty() {
            return Vec::new();
        }
        
        // First, split the token into individual characters
        let mut parts: Vec<Cow<'a, str>> = token.chars().map(|c| Cow::Owned(c.to_string())).collect();
        
        // Short-circuit for single character tokens
        if parts.len() <= 1 {
            return parts;
        }
        
        // Get the static list of merge pairs
        let merges = MERGES;
        
        // Continue merging until no more merges can be applied
        'outer: while parts.len() > 1 {
            let mut best_merge: Option<(usize, usize)> = None;
            let mut best_rank = usize::MAX;
            
            // Find the best merge
            for i in 0..parts.len() - 1 {
                let first = parts[i].as_ref();
                let second = parts[i+1].as_ref();
                
                // Look for matching merge in the static merge list
                for &(merge_first, merge_second, rank) in merges {
                    if first == merge_first && second == merge_second && rank < best_rank {
                        best_rank = rank;
                        best_merge = Some((i, i + 1));
                        
                        // Optimization: if we find the highest priority merge (rank 0), 
                        // we can stop searching
                        if rank == 0 {
                            break;
                        }
                    }
                }
            }
            
            // If no valid merges found, we're done
            if best_merge.is_none() {
                break 'outer;
            }
            
            // Apply the best merge
            let (first_idx, second_idx) = best_merge.unwrap();
            let merged = format!("{}{}", parts[first_idx], parts[second_idx]);
            parts[first_idx] = Cow::Owned(merged);
            parts.remove(second_idx);
        }
        
        parts
    }
}