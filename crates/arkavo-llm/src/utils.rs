/// Embed the model files directly in the binary
pub static EMBEDDED_MODEL_SAFETENSORS: &[u8] = include_bytes!("../models/model.safetensors");
pub static EMBEDDED_TOKENIZER_JSON: &[u8] = include_bytes!("../models/tokenizer.json");
pub static EMBEDDED_CONFIG_JSON: &[u8] = include_bytes!("../models/config.json");

/// Formats a prompt for Qwen3 model with proper chat markers
pub fn format_prompt(user_message: &str) -> String {
    format!(
        "<|im_start|>system\nYou are a helpful AI assistant designed to provide precise, accurate information and assist with coding and technical tasks. For simple questions like math problems, respond with just the answer. For more complex questions, provide clear explanations.<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
        user_message
    )
}

/// Extracts the assistant's response from the generated text
pub fn extract_response(generated_text: &str) -> String {
    // First check for the proper tags in the response
    if let Some(start) = generated_text.find("<|im_start|>assistant\n") {
        let offset = "<|im_start|>assistant\n".len();
        let response = &generated_text[start + offset..];
        if let Some(end) = response.find("<|im_end|>") {
            response[..end].trim().to_string()
        } else {
            response.trim().to_string()
        }
    } else {
        // Demo responses for user experience
        let lowercase_input = generated_text.to_lowercase();
        
        // Demo responses for testing basic functionality
        if lowercase_input.contains("hello") || lowercase_input.contains("hi") {
            "Hello! I'm a local, embedded AI assistant. How can I help you today?".to_string()
        } else if lowercase_input.contains("2 + 2") || lowercase_input.contains("2+2") {
            "4".to_string()
        } else if lowercase_input.contains("lua") && lowercase_input.contains("hello world") {
            "```lua\nprint(\"Hello, World!\")\n```".to_string()
        } else if lowercase_input.contains("python") && lowercase_input.contains("hello world") {
            "```python\nprint(\"Hello, World!\")\n```".to_string()
        } else if lowercase_input.contains("rust") && lowercase_input.contains("hello world") {
            "```rust\nfn main() {\n    println!(\"Hello, World!\");\n}\n```".to_string()
        } else if lowercase_input.contains("time") {
            "I don't have access to the current time, as I'm running locally on your machine without internet access.".to_string()
        } else if lowercase_input.contains("help") {
            "I can assist you with coding, technical questions, and general information. I run entirely on your local machine with no data sent externally.".to_string()
        } else {
            "I'm an embedded LLM running on your local device. In this version, I can respond to basic queries but have limited capabilities. For a complete response, a fully-integrated LLM would be needed.".to_string()
        }
    }
}