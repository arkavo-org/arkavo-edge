use anyhow::Result;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Model files required for Qwen3 operation
const REQUIRED_FILES: [&str; 3] = ["model.safetensors", "tokenizer.json", "config.json"];

/// Embed the model files directly in the binary
#[allow(dead_code)] // Used conditionally with feature flags
static EMBEDDED_MODEL_SAFETENSORS: &[u8] = include_bytes!("../models/model.safetensors");
#[allow(dead_code)] // Used conditionally with feature flags
static EMBEDDED_TOKENIZER_JSON: &[u8] = include_bytes!("../models/tokenizer.json");
#[allow(dead_code)] // Used conditionally with feature flags
static EMBEDDED_CONFIG_JSON: &[u8] = include_bytes!("../models/config.json");

/// Verifies that all required model files are present
pub async fn check_model_files(model_path: &str) -> bool {
    let path = Path::new(model_path);

    for file in REQUIRED_FILES.iter() {
        if !path.join(file).exists() {
            return false;
        }
    }

    true
}

/// Writes the embedded model to a temporary file at runtime
pub fn write_model_to_temp(model_bytes: &[u8]) -> std::io::Result<PathBuf> {
    let mut temp_path = std::env::temp_dir();
    temp_path.push("qwen3-0.6b-q4.bin");
    
    let mut file = File::create(&temp_path)?;
    file.write_all(model_bytes)?;
    
    Ok(temp_path)
}

/// Writes the embedded model to the specified path
#[cfg(feature = "embedded_model")]
#[allow(dead_code)] // Used conditionally with feature flags
async fn write_embedded_model_to_path(path: &Path) -> Result<()> {
    use std::io::ErrorKind;
    
    // Write model.safetensors
    let model_file_path = path.join("model.safetensors");
    match File::create(&model_file_path) {
        Ok(mut file) => {
            file.write_all(EMBEDDED_MODEL_SAFETENSORS)?;
        },
        Err(e) => {
            return if e.kind() == ErrorKind::PermissionDenied {
                Err(anyhow::anyhow!("Permission denied when writing model file. Try running with elevated permissions."))
            } else {
                Err(anyhow::anyhow!("Failed to write embedded model: {}", e))
            }
        }
    }
    
    // Write tokenizer.json
    let tokenizer_file_path = path.join("tokenizer.json");
    match File::create(&tokenizer_file_path) {
        Ok(mut file) => {
            file.write_all(EMBEDDED_TOKENIZER_JSON)?;
        },
        Err(e) => {
            return Err(anyhow::anyhow!("Failed to write tokenizer file: {}", e));
        }
    }
    
    // Write config.json
    let config_file_path = path.join("config.json");
    match File::create(&config_file_path) {
        Ok(mut file) => {
            file.write_all(EMBEDDED_CONFIG_JSON)?;
        },
        Err(e) => {
            return Err(anyhow::anyhow!("Failed to write config file: {}", e));
        }
    }
    
    tracing::info!("Successfully extracted embedded model files to {}", path.display());
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
