use crate::{Error, Result};
use tokenizers::tokenizer::Tokenizer as HfTokenizer;

pub struct Tokenizer {
    inner: Option<HfTokenizer>,
    model_name: String,
}

impl Tokenizer {
    pub fn new(model_name: &str) -> Result<Self> {
        // For now, we'll load tokenizers from local files or download them
        // In the future, this will be integrated with the model manager
        Ok(Self {
            inner: None,
            model_name: model_name.to_string(),
        })
    }

    pub async fn load(&mut self) -> Result<()> {
        // Placeholder for tokenizer loading
        tracing::info!("Loading tokenizer for model '{}'", self.model_name);

        // TODO: Implement actual tokenizer loading from:
        // 1. Local cache
        // 2. Hugging Face hub
        // 3. Bundled tokenizers

        Ok(())
    }

    pub fn encode(&self, text: &str) -> Result<Vec<u32>> {
        if let Some(tokenizer) = &self.inner {
            let encoding = tokenizer
                .encode(text, false)
                .map_err(|e| Error::Model(format!("Tokenization failed: {}", e)))?;
            Ok(encoding.get_ids().to_vec())
        } else {
            // Fallback for when tokenizer isn't loaded
            Ok(text.chars().map(|c| c as u32).collect())
        }
    }

    pub fn decode(&self, tokens: &[u32]) -> Result<String> {
        if let Some(tokenizer) = &self.inner {
            tokenizer
                .decode(tokens, true)
                .map_err(|e| Error::Model(format!("Decoding failed: {}", e)))
        } else {
            // Fallback for when tokenizer isn't loaded
            Ok(tokens
                .iter()
                .filter_map(|&t| std::char::from_u32(t))
                .collect())
        }
    }
}
