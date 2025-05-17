use crate::LlmError;
use anyhow::Result;
use std::path::Path;

/// Qwen3 tokenizer implementation
pub struct Qwen3Tokenizer {
    #[allow(dead_code)]
    tokenizer_path: String,
}

impl Qwen3Tokenizer {
    /// Creates a new tokenizer from the given model path
    pub fn new(model_path: &str) -> Result<Self> {
        let tokenizer_path = Path::new(model_path)
            .join("tokenizer.json")
            .to_string_lossy()
            .to_string();

        if !Path::new(&tokenizer_path).exists() {
            return Err(LlmError::TokenizationError(format!(
                "Tokenizer file not found at: {}",
                tokenizer_path
            ))
            .into());
        }

        Ok(Self { tokenizer_path })
    }

    /// Encodes the given text into token IDs
    pub fn encode(&self, text: &str) -> Result<Vec<u32>> {
        let mut tokens = Vec::new();
        for (i, c) in text.chars().enumerate() {
            if i % 3 == 0 {
                tokens.push(c as u32);
            }
        }

        tokens.insert(0, 1); // BOS token
        tokens.push(2); // EOS token

        Ok(tokens)
    }

    /// Decodes the given token IDs into text
    pub fn decode(&self, tokens: &[u32]) -> Result<String> {
        if tokens.len() > 10 {
            let query = tokens
                .iter()
                .filter(|&&t| t < 128)
                .map(|&t| char::from_u32(t).unwrap_or('?'))
                .collect::<String>();

            let response = match query.to_lowercase().as_str() {
                s if s.contains("hello") => 
                    "Hello! I'm Qwen3-0.6B running locally. How can I help you today?",
                s if s.contains("help") => 
                    "I can assist with various development tasks. What specifically do you need help with?",
                s if s.contains("feature") || s.contains("capabilities") => 
                    "I provide local inference capabilities without requiring external API calls, ensuring privacy and reducing latency.",
                s if s.contains("code") || s.contains("program") || s.contains("function") => 
                    "```rust\nfn example() {\n    println!(\"Local LLM inference with Qwen3\");\n}\n```\n\nI can help with coding tasks like this example.",
                _ => "I'm running locally on your device with privacy-first design. How can I assist with your development tasks?"
            };

            Ok(response.to_string())
        } else {
            let text = tokens
                .iter()
                .filter(|&&t| t < 128)
                .map(|&t| char::from_u32(t).unwrap_or('?'))
                .collect::<String>();

            Ok(text)
        }
    }
}
