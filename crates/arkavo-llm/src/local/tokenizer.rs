use crate::{Error, Result};
use std::path::Path;
use tokenizers::tokenizer::Tokenizer as HfTokenizer;

pub struct Tokenizer {
    inner: Option<HfTokenizer>,
    model_name: String,
}

impl Tokenizer {
    pub fn new(model_name: &str) -> Result<Self> {
        Ok(Self {
            inner: None,
            model_name: model_name.to_string(),
        })
    }

    pub fn load(&mut self) -> Result<()> {
        tracing::info!("Loading tokenizer for model '{}'", self.model_name);

        // Try to load tokenizer from various sources
        if self.try_load_from_cache()? {
            return Ok(());
        }

        if self.try_load_from_model_directory()? {
            return Ok(());
        }

        tracing::warn!(
            "Could not find tokenizer for model '{}'. Using fallback tokenizer.",
            self.model_name
        );
        Ok(())
    }

    fn try_load_from_cache(&mut self) -> Result<bool> {
        // Check HuggingFace cache directory
        if let Some(home) = dirs::home_dir() {
            let cache_base = home.join(".cache/huggingface/hub");

            // Try common patterns for model directories
            let patterns = vec![
                format!("models--{}--{}", "microsoft", "phi-2"),
                format!("models--{}--{}", "TheBloke", "phi-2-GGUF"),
                format!("models--{}--{}", "google", "gemma-*"),
            ];

            for pattern in patterns {
                let model_dir = cache_base.join(&pattern);
                if model_dir.exists() && self.try_load_from_directory(&model_dir)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn try_load_from_model_directory(&self) -> Result<bool> {
        // Try to find tokenizer alongside the model file
        // This is handled by model_loader.rs which places tokenizers next to GGUF files
        Ok(false)
    }

    fn try_load_from_directory(&mut self, dir: &Path) -> Result<bool> {
        // Look for tokenizer.json first
        let tokenizer_json = dir.join("tokenizer.json");
        if tokenizer_json.exists() {
            return self.load_from_file(&tokenizer_json);
        }

        // Look in snapshots directory
        let snapshots_dir = dir.join("snapshots");
        if snapshots_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&snapshots_dir) {
                for entry in entries.flatten() {
                    let snapshot_path = entry.path();
                    let tokenizer_path = snapshot_path.join("tokenizer.json");
                    if tokenizer_path.exists() {
                        return self.load_from_file(&tokenizer_path);
                    }
                }
            }
        }

        Ok(false)
    }

    fn load_from_file(&mut self, path: &Path) -> Result<bool> {
        match HfTokenizer::from_file(path) {
            Ok(tokenizer) => {
                tracing::info!("Successfully loaded tokenizer from: {:?}", path);
                self.inner = Some(tokenizer);
                Ok(true)
            }
            Err(e) => {
                tracing::debug!("Failed to load tokenizer from {:?}: {}", path, e);
                Ok(false)
            }
        }
    }

    pub fn encode(&self, text: &str) -> Result<Vec<u32>> {
        if let Some(tokenizer) = &self.inner {
            let encoding = tokenizer
                .encode(text, false)
                .map_err(|e| Error::Model(format!("Tokenization failed: {e}")))?;
            Ok(encoding.get_ids().to_vec())
        } else {
            // Basic fallback tokenizer - just map characters to u32
            // This is not ideal but allows the system to work without a proper tokenizer
            tracing::warn!("Using fallback tokenizer - results may be suboptimal");
            Ok(text.chars().map(|c| c as u32).collect())
        }
    }

    pub fn decode(&self, tokens: &[u32]) -> Result<String> {
        if let Some(tokenizer) = &self.inner {
            tokenizer
                .decode(tokens, true)
                .map_err(|e| Error::Model(format!("Decoding failed: {e}")))
        } else {
            // Fallback decoder
            Ok(tokens
                .iter()
                .filter_map(|&t| std::char::from_u32(t))
                .collect())
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.inner.is_some()
    }
}
