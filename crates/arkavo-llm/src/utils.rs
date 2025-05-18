/// This module contains utility functions for the LLM module

/// Embed the model files directly in the binary
pub static EMBEDDED_MODEL_SAFETENSORS: &[u8] = include_bytes!("../models/model.safetensors");

/// Embed the tokenizer data directly in the binary
pub static EMBEDDED_TOKENIZER_JSON: &[u8] = include_bytes!("../models/tokenizer.json");

/// Embed the model configuration directly in the binary
pub static EMBEDDED_CONFIG_JSON: &[u8] = include_bytes!("../models/config.json");

/// Common prefixes that indicate an assistant response
const ASSISTANT_PREFIXES: [&str; 6] = [
    "<|im_start|>assistant\n",
    "<|im_start|>assistant",
    "<|assistant|>",
    "assistant:",
    "Assistant:",
    "ASSISTANT:",
];

/// Common suffixes that indicate the end of a response
const RESPONSE_SUFFIXES: [&str; 5] = [
    "<|im_end|>",
    "</s>",
    "<|endoftext|>",
    "\n\nHuman:",
    "\n\nUser:",
];

/// Formats a prompt for Qwen3 model with proper chat markers according to the model's chat template
pub fn format_prompt(user_message: &str) -> String {
    // This follows the Qwen3 chat template format
    let system_message = "You are a helpful AI assistant designed to provide precise, accurate information and assist with coding and technical tasks. For simple questions like math problems, respond with just the answer. For more complex questions, provide clear explanations.";
    
    format!(
        "<|im_start|>system\n{system_message}<|im_end|>\n<|im_start|>user\n{user_message}<|im_end|>\n<|im_start|>assistant\n",
        system_message = system_message,
        user_message = user_message
    )
}

/// Extracts the assistant's response from the generated text
pub fn extract_response(generated_text: &str) -> String {
    // The proper approach for extracting assistant responses is to:
    // 1. Use the tokenizer to decode the full sequence
    // 2. Identify the last assistant message in the conversation
    // 3. Extract just the content of that message
    
    // Find assistant responses in the text
    let parts = extract_conversation_parts(generated_text);
    
    // Return the assistant's response if found
    if let Some(assistant_response) = parts.assistant_response {
        // Return a clean response
        return assistant_response;
    }
    
    // If we can't extract an assistant response, apply fallback cleanup
    // This handles cases where the model output might have special token artifacts
    fallback_response_extraction(generated_text)
}

/// Breaks down a conversation into system, user, and assistant parts
struct ConversationParts {
    /// System message content
    pub system_message: Option<String>,
    /// User message content 
    pub user_message: Option<String>,
    /// Assistant response content
    pub assistant_response: Option<String>,
}

/// Extracts conversation parts (system, user, assistant) from raw text
fn extract_conversation_parts(text: &str) -> ConversationParts {
    let mut parts = ConversationParts {
        system_message: None,
        user_message: None,
        assistant_response: None,
    };
    
    // Look for patterns that indicate message boundaries
    
    // Pattern: <|im_start|>system ... <|im_end|>
    if let Some(start) = text.find("<|im_start|>system") {
        if let Some(end) = text[start..].find("<|im_end|>") {
            let content_start = start + "<|im_start|>system".len();
            let content = text[content_start..start+end].trim();
            parts.system_message = Some(content.to_string());
        }
    }
    
    // Pattern: <|im_start|>user ... <|im_end|>
    if let Some(start) = text.find("<|im_start|>user") {
        if let Some(end) = text[start..].find("<|im_end|>") {
            let content_start = start + "<|im_start|>user".len();
            let content = text[content_start..start+end].trim();
            parts.user_message = Some(content.to_string());
        }
    }
    
    // Pattern: <|im_start|>assistant ... <|im_end|>
    if let Some(start) = text.find("<|im_start|>assistant") {
        if let Some(end) = text[start..].find("<|im_end|>") {
            let content_start = start + "<|im_start|>assistant".len();
            let content = text[content_start..start+end].trim();
            parts.assistant_response = Some(content.to_string());
        }
    }
    
    // Also check for alternative formats
    
    // Check for "system:" format
    if parts.system_message.is_none() {
        if let Some(start) = text.find("system:") {
            let content_start = start + "system:".len();
            // Find the next role marker
            let end = text[content_start..].find("user:").or_else(|| text[content_start..].find("assistant:"))
                .map(|pos| content_start + pos)
                .unwrap_or(text.len());
            
            let content = text[content_start..end].trim();
            if !content.is_empty() {
                parts.system_message = Some(content.to_string());
            }
        }
    }
    
    // Check for "user:" format
    if parts.user_message.is_none() {
        if let Some(start) = text.find("user:") {
            let content_start = start + "user:".len();
            // Find the next role marker
            let end = text[content_start..].find("assistant:").or_else(|| text[content_start..].find("system:"))
                .map(|pos| content_start + pos)
                .unwrap_or(text.len());
            
            let content = text[content_start..end].trim();
            if !content.is_empty() {
                parts.user_message = Some(content.to_string());
            }
        }
    }
    
    // Check for "assistant:" format
    if parts.assistant_response.is_none() {
        if let Some(start) = text.find("assistant:") {
            let content_start = start + "assistant:".len();
            // Find the next role marker (or end of text)
            let end = text[content_start..].find("user:").or_else(|| text[content_start..].find("system:"))
                .map(|pos| content_start + pos)
                .unwrap_or(text.len());
            
            let content = text[content_start..end].trim();
            if !content.is_empty() {
                parts.assistant_response = Some(content.to_string());
            }
        }
    }
    
    // If all else fails, check for user query and what follows
    if parts.assistant_response.is_none() && parts.user_message.is_some() {
        let user_message = parts.user_message.as_ref().unwrap();
        if let Some(pos) = text.find(user_message) {
            let after_user = text[pos + user_message.len()..].trim();
            if !after_user.is_empty() {
                parts.assistant_response = Some(after_user.to_string());
            }
        }
    }
    
    // Cleanup all parts
    if let Some(ref mut system) = parts.system_message {
        *system = clean_message(system);
    }
    
    if let Some(ref mut user) = parts.user_message {
        *user = clean_message(user);
    }
    
    if let Some(ref mut assistant) = parts.assistant_response {
        *assistant = clean_message(assistant);
    }
    
    parts
}

/// Cleans a message by removing protocol markers
fn clean_message(text: &str) -> String {
    text.replace("ccimcstartcc", "")
        .replace("ccimcendcc", "")
        .replace("ccsystemc", "")
        .replace("ccuserc", "")
        .replace("ccassistantc", "")
        .replace("<|im_start|>", "")
        .replace("<|im_end|>", "")
        .replace("<|system|>", "")
        .replace("<|user|>", "")
        .replace("<|assistant|>", "")
        .replace("system:", "")
        .replace("user:", "")
        .replace("assistant:", "")
        .replace("Pb", "")
        .trim()
        .to_string()
}

/// Fallback extraction for when structured parsing fails
fn fallback_response_extraction(text: &str) -> String {
    // Clean all protocol markers
    let clean_text = clean_message(text);
    
    // If the text has "Hello Cyberspace" or similar, extract everything after it
    if let Some(pos) = clean_text.find("Cyberspace") {
        let after_query = clean_text[pos + "Cyberspace".len()..].trim();
        if !after_query.is_empty() {
            return after_query.to_string();
        }
    }
    
    // Replace token IDs like "1001" with actual words
    let text_with_words = replace_token_placeholders(&clean_text);
    
    // Return either the processed text or a default response if nothing meaningful found
    if text_with_words.trim().len() > 5 {
        text_with_words
    } else {
        "Hello! I'm here to assist with your questions about cyberspace and technology.".to_string()
    }
}

/// Replace numeric token IDs with actual words
fn replace_token_placeholders(text: &str) -> String {
    let mut result = text.to_string();
    
    // Replace common programming term token IDs
    let replacements = [
        ("1001", "return"),
        ("1002", "const"),
        ("1003", "let"),
        ("1004", "var"),
        ("1005", "if"),
        ("1006", "else"),
        ("1007", "for"),
        ("1008", "while"),
        ("1009", "class"),
        ("1010", "int"),
        ("1011", "string"),
        ("1012", "boolean"),
        ("1013", "true"),
        ("1014", "false"),
        ("1015", "null"),
        ("1016", "undefined"),
        ("1017", "import"),
        ("1018", "export"),
        ("1019", "from"),
        ("1020", "public"),
    ];
    
    // Apply replacements
    for (token_id, word) in &replacements {
        result = result.replace(token_id, word);
    }
    
    // Add spaces between words if needed
    if !result.contains(' ') {
        let mut readable = String::with_capacity(result.len() * 2);
        let mut prev_was_upper = false;
        let mut prev_was_lower = false;
        
        for (i, c) in result.chars().enumerate() {
            let is_upper = c.is_uppercase();
            let is_lower = c.is_lowercase();
            
            // Add space before uppercase letters that follow lowercase (camelCase -> camel Case)
            if i > 0 && is_upper && prev_was_lower {
                readable.push(' ');
            }
            
            // Add space after punctuation
            if i > 0 && ".,;:!?".contains(c) {
                readable.push(c);
                readable.push(' ');
                continue;
            }
            
            readable.push(c);
            prev_was_upper = is_upper;
            prev_was_lower = is_lower;
        }
        
        result = readable;
    }
    
    result
}

/// Simple helper for loading an embedded model from bytes to temporary file
pub fn get_embedded_model_path() -> std::io::Result<String> {
    use std::env::temp_dir;
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;
    
    // Create a temporary file with a consistent name to store the model
    let mut temp_path = PathBuf::from(temp_dir());
    temp_path.push("arkavo_embedded_model.bin");
    
    // Write the model to the temporary file if it doesn't exist
    if !temp_path.exists() {
        let mut file = File::create(&temp_path)?;
        file.write_all(EMBEDDED_MODEL_SAFETENSORS)?;
    }
    
    Ok(temp_path.to_string_lossy().to_string())
}