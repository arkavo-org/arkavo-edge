//! Embedding service using bundled model for offline text embeddings
//!
//! This module provides a thread-safe embedding service that uses a bundled
//! AllMiniLML6V2 model for completely offline operation. The model files are
//! embedded at compile time, ensuring no runtime downloads are needed.
//!
//! # Thread Safety
//!
//! The `EmbeddingService` is thread-safe and can be shared across multiple
//! async tasks. The actual embedding generation runs in a blocking thread pool
//! to avoid blocking the async runtime, as fastembed operations are synchronous.
//!
//! Multiple concurrent embedding requests will be serialized through the RwLock,
//! but the blocking operations won't block other async tasks.

use crate::error::{MemoryError, Result};
use fastembed::{
    InitOptionsUserDefined, Pooling, QuantizationMode, TextEmbedding, TokenizerFiles,
    UserDefinedEmbeddingModel,
};
use std::sync::Arc;
use tokio::sync::RwLock;

// Embed model files at compile time
const MODEL_ONNX: &[u8] = include_bytes!("../models/model.onnx");
const TOKENIZER_JSON: &[u8] = include_bytes!("../models/tokenizer.json");
const CONFIG_JSON: &[u8] = include_bytes!("../models/config.json");
const SPECIAL_TOKENS_MAP_JSON: &[u8] = include_bytes!("../models/special_tokens_map.json");
const TOKENIZER_CONFIG_JSON: &[u8] = include_bytes!("../models/tokenizer_config.json");

/// Thread-safe embedding service using bundled model
pub struct EmbeddingService {
    model: Arc<RwLock<Option<TextEmbedding>>>,
}

impl Default for EmbeddingService {
    fn default() -> Self {
        Self::new()
    }
}

impl EmbeddingService {
    pub fn new() -> Self {
        Self {
            model: Arc::new(RwLock::new(None)),
        }
    }

    /// Initialize the embedding model lazily from bundled files
    async fn ensure_initialized(&self) -> Result<()> {
        let is_initialized = {
            let model_guard = self.model.read().await;
            model_guard.is_some()
        };

        if !is_initialized {
            let mut model_guard = self.model.write().await;
            // Double-check in case another task initialized it
            if model_guard.is_none() {
                log::info!("Initializing bundled embedding model (AllMiniLML6V2)");

                // Run initialization in blocking thread
                let text_embedding = tokio::task::spawn_blocking(move || {
                    // Create TokenizerFiles from embedded data
                    let tokenizer_files = TokenizerFiles {
                        tokenizer_file: TOKENIZER_JSON.to_vec(),
                        config_file: CONFIG_JSON.to_vec(),
                        special_tokens_map_file: SPECIAL_TOKENS_MAP_JSON.to_vec(),
                        tokenizer_config_file: TOKENIZER_CONFIG_JSON.to_vec(),
                    };

                    // Create UserDefinedEmbeddingModel with embedded ONNX model
                    let user_defined_model =
                        UserDefinedEmbeddingModel::new(MODEL_ONNX.to_vec(), tokenizer_files)
                            .with_pooling(Pooling::Mean) // AllMiniLML6V2 uses mean pooling
                            .with_quantization(QuantizationMode::None); // No quantization for base model

                    // Create init options
                    let options = InitOptionsUserDefined::new().with_max_length(384); // AllMiniLML6V2 max length

                    TextEmbedding::try_new_from_user_defined(user_defined_model, options)
                })
                .await
                .map_err(|e| {
                    MemoryError::ModelNotAvailable(format!(
                        "Failed to spawn initialization task: {}",
                        e
                    ))
                })?
                .map_err(|e| {
                    MemoryError::ModelNotAvailable(format!(
                        "Failed to initialize embedding model: {}",
                        e
                    ))
                })?;

                *model_guard = Some(text_embedding);
                log::info!("Bundled embedding model initialized successfully");
            }
        }
        Ok(())
    }

    pub async fn ensure_model_available(&self) -> Result<()> {
        self.ensure_initialized().await
    }

    pub async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        // Ensure model is initialized
        self.ensure_initialized().await?;

        // Clone the text to move into the blocking task
        let text = text.to_string();

        // Clone the model Arc for the blocking task
        let model_clone = self.model.clone();

        // Run the blocking embed operation in a separate thread
        let embeddings = tokio::task::spawn_blocking(move || {
            let model_guard = model_clone.blocking_read();
            let model = model_guard.as_ref().ok_or_else(|| {
                MemoryError::ModelNotAvailable("Model not initialized".to_string())
            })?;

            // Generate embeddings
            let documents = vec![text.as_str()];
            let embeddings = model.embed(documents, None).map_err(|e| {
                MemoryError::Embedding(format!("Failed to generate embedding: {}", e))
            })?;

            // Extract the first (and only) embedding
            let embedding = embeddings
                .into_iter()
                .next()
                .ok_or_else(|| MemoryError::Embedding("No embedding generated".to_string()))?;

            Ok::<Vec<f32>, MemoryError>(embedding)
        })
        .await
        .map_err(|e| MemoryError::Embedding(format!("Task join error: {}", e)))??;

        Ok(embeddings)
    }

    pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        let mut dot_product = 0.0;
        let mut norm_a = 0.0;
        let mut norm_b = 0.0;

        for i in 0..a.len().min(b.len()) {
            dot_product += a[i] * b[i];
            norm_a += a[i] * a[i];
            norm_b += b[i] * b[i];
        }

        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }

        dot_product / (norm_a.sqrt() * norm_b.sqrt())
    }

    /// Get embedding dimension for the current model
    pub fn embedding_dimension(&self) -> usize {
        // AllMiniLML6V2 produces 384-dimensional embeddings
        384
    }
}

/// Available embedding models (currently only bundled AllMiniLML6V2)
pub fn list_available_models() -> Vec<String> {
    vec!["AllMiniLML6V2 (bundled)".to_string()]
}

