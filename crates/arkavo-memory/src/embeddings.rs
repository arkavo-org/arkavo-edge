//! Embedding service using fastembed for text embeddings
//! 
//! This module provides a thread-safe embedding service that lazily initializes
//! the embedding model on first use. The model is downloaded automatically
//! and cached locally.
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
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Thread-safe embedding service using fastembed
pub struct EmbeddingService {
    model: Arc<RwLock<Option<TextEmbedding>>>,
    model_type: EmbeddingModel,
}

impl Default for EmbeddingService {
    fn default() -> Self {
        Self::new()
    }
}

impl EmbeddingService {
    pub fn new() -> Self {
        // Use AllMiniLML6V2 as the default model
        Self::with_model(EmbeddingModel::AllMiniLML6V2)
    }

    pub fn with_model(model_type: EmbeddingModel) -> Self {
        Self {
            model: Arc::new(RwLock::new(None)),
            model_type,
        }
    }

    /// Initialize the embedding model lazily
    async fn ensure_initialized(&self) -> Result<()> {
        let is_initialized = {
            let model_guard = self.model.read().await;
            model_guard.is_some()
        };
        
        if !is_initialized {
            let mut model_guard = self.model.write().await;
            // Double-check in case another task initialized it
            if model_guard.is_none() {
                log::info!("Initializing embedding model: {:?}", self.model_type);
                
                let model_type = self.model_type.clone();
                
                // Run initialization in blocking thread since it may download models
                let text_embedding = tokio::task::spawn_blocking(move || {
                    // Set cache directory to .arkavo/fastembed_cache
                    let cache_dir = std::path::PathBuf::from(".arkavo").join("fastembed_cache");
                    std::fs::create_dir_all(&cache_dir).ok();
                    
                    let init_options = InitOptions::new(model_type)
                        .with_cache_dir(cache_dir)
                        .with_show_download_progress(true);
                    
                    TextEmbedding::try_new(init_options)
                })
                .await
                .map_err(|e| MemoryError::ModelNotAvailable(format!(
                    "Failed to spawn initialization task: {}", e
                )))?
                .map_err(|e| MemoryError::ModelNotAvailable(format!(
                    "Failed to initialize embedding model: {}", e
                )))?;
                
                *model_guard = Some(text_embedding);
                log::info!("Embedding model initialized successfully");
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
            let model = model_guard.as_ref()
                .ok_or_else(|| MemoryError::ModelNotAvailable("Model not initialized".to_string()))?;
            
            // Generate embeddings
            let documents = vec![text.as_str()];
            let embeddings = model.embed(documents, None)
                .map_err(|e| MemoryError::Embedding(format!("Failed to generate embedding: {}", e)))?;
            
            // Extract the first (and only) embedding
            let embedding = embeddings.into_iter()
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
        // fastembed default model (AllMiniLML6V2) produces 384-dimensional embeddings
        384
    }
}

/// Available embedding models
pub fn list_available_models() -> Vec<String> {
    TextEmbedding::list_supported_models()
        .into_iter()
        .map(|m| format!("{:?}", m))
        .collect()
}