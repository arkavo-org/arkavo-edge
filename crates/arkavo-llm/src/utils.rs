use anyhow::Result;
use std::path::Path;

/// Model files required for Qwen3 operation
const REQUIRED_FILES: [&str; 3] = ["model.safetensors", "tokenizer.json", "config.json"];

/// Verifies that all required model files are present
pub async fn check_model_files(model_path: &str) -> bool {
    let path = Path::new(model_path);

    // Check each required file
    for file in REQUIRED_FILES.iter() {
        if !path.join(file).exists() {
            return false;
        }
    }

    true
}

/// Downloads the Qwen3 model files from the repository
pub async fn download_model(model_path: &str) -> Result<()> {
    // Create the model directory if it doesn't exist
    let path = Path::new(model_path);
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }

    // Create placeholder files for early development
    // In production, this would download actual model files
    for file in REQUIRED_FILES.iter() {
        let file_path = path.join(file);
        std::fs::write(&file_path, "placeholder content")?;
    }

    Ok(())
}

/// Formats a prompt for Qwen3 model with proper chat markers
pub fn format_prompt(user_message: &str) -> String {
    format!(
        "<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
        user_message
    )
}

/// Extracts the assistant's response from the generated text
pub fn extract_response(generated_text: &str) -> String {
    if let Some(start) = generated_text.find("<|im_start|>assistant\n") {
        let offset = "<|im_start|>assistant\n".len();
        let response = &generated_text[start + offset..];
        if let Some(end) = response.find("<|im_end|>") {
            return response[..end].trim().to_string();
        }
        return response.trim().to_string();
    }

    generated_text.trim().to_string()
}
