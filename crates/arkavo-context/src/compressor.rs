use crate::{Error, Result};
#[cfg(feature = "llm-local")]
use arkavo_llm::local::LocalProvider;
use arkavo_llm::{Message, Provider, Role};
use std::sync::Arc;
use tokio::sync::Mutex;

#[cfg(feature = "llm-local")]
pub struct ContextCompressor {
    provider: Arc<Mutex<LocalProvider>>,
    model_name: String,
}

#[cfg(not(feature = "llm-local"))]
pub struct ContextCompressor {
    _phantom: std::marker::PhantomData<()>,
}

#[cfg(feature = "llm-local")]
impl ContextCompressor {
    pub async fn new(model_name: String, model_path: Option<String>) -> Result<Self> {
        let provider = LocalProvider::new(model_name.clone(), model_path)?;
        provider.initialize().await?;

        Ok(Self {
            provider: Arc::new(Mutex::new(provider)),
            model_name,
        })
    }

    pub async fn compress(&self, text: &str, target_ratio: f64) -> Result<String> {
        if text.is_empty() {
            return Ok(String::new());
        }

        let target_words = ((text.split_whitespace().count() as f64) * target_ratio) as usize;

        let prompt = self.create_compression_prompt(text, target_words);

        let messages = vec![Message {
            role: Role::User,
            content: prompt,
            images: None,
        }];

        let compressed = self
            .provider
            .lock()
            .await
            .complete(messages)
            .await
            .map_err(|e| Error::Model(format!("Compression failed: {e}")))?;

        Ok(compressed.trim().to_string())
    }

    fn create_compression_prompt(&self, text: &str, target_words: usize) -> String {
        format!(
            "Summarize the following text to approximately {target_words} words. \
            Preserve key information, technical terms, and important details. \
            Focus on facts and remove redundancy.\n\nText:\n{text}\n\nSummary:"
        )
    }

    pub async fn compress_to_tokens(&self, text: &str, target_tokens: u32) -> Result<String> {
        let words_per_token = 0.75;
        let target_words = (target_tokens as f64 * words_per_token) as usize;

        let prompt = format!(
            "Compress the following text to approximately {target_words} words (~{target_tokens} tokens). \
            Keep critical information, preserve technical accuracy, remove redundancy.\n\nText:\n{text}\n\nCompressed:"
        );

        let messages = vec![Message {
            role: Role::User,
            content: prompt,
            images: None,
        }];

        let compressed = self
            .provider
            .lock()
            .await
            .complete(messages)
            .await
            .map_err(|e| Error::Model(format!("Token-based compression failed: {e}")))?;

        Ok(compressed.trim().to_string())
    }

    pub fn model_name(&self) -> &str {
        &self.model_name
    }
}

#[cfg(not(feature = "llm-local"))]
impl ContextCompressor {
    pub async fn new(_model_name: String, _model_path: Option<String>) -> Result<Self> {
        Err(Error::Model(
            "ContextCompressor requires llm-local feature".to_string(),
        ))
    }

    pub async fn compress(&self, _text: &str, _target_ratio: f64) -> Result<String> {
        Err(Error::Model(
            "ContextCompressor requires llm-local feature".to_string(),
        ))
    }

    pub async fn compress_to_tokens(&self, _text: &str, _target_tokens: u32) -> Result<String> {
        Err(Error::Model(
            "ContextCompressor requires llm-local feature".to_string(),
        ))
    }

    pub fn model_name(&self) -> &str {
        ""
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_empty_text() {
        let compressor = ContextCompressor::new(
            "gemma-3-270m-it".to_string(),
            Some("unsloth/gemma-3-270m-it-GGUF".to_string()),
        )
        .await;

        if compressor.is_err() {
            eprintln!("Skipping test: Local model not available");
            return;
        }
        let compressor = compressor.unwrap();

        let result = compressor.compress("", 0.5).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "");
    }

    #[test]
    fn test_compression_prompt_format() {
        let compressor_result = tokio::runtime::Runtime::new().unwrap().block_on(async {
            ContextCompressor::new(
                "gemma-3-270m-it".to_string(),
                Some("unsloth/gemma-3-270m-it-GGUF".to_string()),
            )
            .await
        });

        if let Ok(compressor) = compressor_result {
            let prompt = compressor.create_compression_prompt("Test text here", 50);
            assert!(prompt.contains("50 words"));
            assert!(prompt.contains("Test text here"));
        }
    }
}
