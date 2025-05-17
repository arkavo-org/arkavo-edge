/// This module contains utility functions for the LLM module

/// Embed the model files directly in the binary
#[cfg(feature = "embedded_model")]
pub static EMBEDDED_MODEL_SAFETENSORS: &[u8] = include_bytes!("../models/model.safetensors");

/// Embed the tokenizer data directly in the binary
pub static EMBEDDED_TOKENIZER_JSON: &[u8] = include_bytes!("../models/tokenizer.json");

/// Embed the model configuration directly in the binary
#[cfg(feature = "embedded_model")]
pub static EMBEDDED_CONFIG_JSON: &[u8] = include_bytes!("../models/config.json");

/// Common prefixes for extracting the relevant response part
const RESPONSE_PREFIXES: [&str; 4] = [
    "<|im_start|>assistant\n",
    "assistant:",
    "Assistant:",
    "ASSISTANT:",
];

/// Common suffixes to trim from responses 
const RESPONSE_SUFFIXES: [&str; 3] = [
    "<|im_end|>",
    "\n\nHuman:",
    "\n\nUser:",
];

/// Formats a prompt for Qwen3 model with proper chat markers
pub fn format_prompt(user_message: &str) -> String {
    format!(
        "<|im_start|>system\nYou are a helpful AI assistant designed to provide precise, accurate information and assist with coding and technical tasks. For simple questions like math problems, respond with just the answer. For more complex questions, provide clear explanations.<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
        user_message
    )
}

/// Extracts the assistant's response from the generated text
pub fn extract_response(generated_text: &str) -> String {
    // First try to extract content based on common prefixes
    for prefix in &RESPONSE_PREFIXES {
        if let Some(start) = generated_text.find(prefix) {
            let offset = prefix.len();
            let mut response = generated_text[start + offset..].trim().to_string();
            
            // Then try to remove common suffixes
            for suffix in &RESPONSE_SUFFIXES {
                if let Some(end) = response.find(suffix) {
                    response = response[..end].trim().to_string();
                    return response;
                }
            }
            
            return response;
        }
    }
    
    // If no prefix found, just return the raw text
    generated_text.trim().to_string()
}

/// Simple helper for loading an embedded model from bytes to temporary file
#[cfg(feature = "embedded_model")]
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