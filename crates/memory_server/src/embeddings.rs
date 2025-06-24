use crate::error::{MemoryError, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
struct EmbeddingRequest {
    model: String,
    prompt: String,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    embedding: Vec<f32>,
}

pub struct EmbeddingService {
    client: Client,
    base_url: String,
    model: String,
}

impl Default for EmbeddingService {
    fn default() -> Self {
        Self::new()
    }
}

impl EmbeddingService {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            base_url: "http://localhost:11434".to_string(),
            model: "nomic-embed-text".to_string(),
        }
    }
    
    pub async fn ensure_model_available(&self) -> Result<()> {
        let response = self.client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
            .map_err(|e| MemoryError::Embedding(format!("Failed to connect to Ollama: {}. Please ensure Ollama is running on localhost:11434", e)))?;
            
        if !response.status().is_success() {
            return Err(MemoryError::Embedding("Failed to list Ollama models".to_string()));
        }
        
        let body: serde_json::Value = response.json().await
            .map_err(|e| MemoryError::Embedding(format!("Failed to parse Ollama response: {}", e)))?;
            
        let models = body["models"].as_array()
            .ok_or_else(|| MemoryError::Embedding("Invalid Ollama response format".to_string()))?;
            
        let has_model = models.iter()
            .any(|m| m["name"].as_str().unwrap_or("").contains(&self.model));
            
        if !has_model {
            return Err(MemoryError::Embedding(format!(
                "Embedding model '{}' not found. Please run: ollama pull {}", 
                self.model, self.model
            )));
        }
        
        Ok(())
    }

    pub async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        let request = EmbeddingRequest {
            model: self.model.clone(),
            prompt: text.to_string(),
        };

        let response = self
            .client
            .post(format!("{}/api/embeddings", self.base_url))
            .json(&request)
            .send()
            .await
            .map_err(|e| MemoryError::Embedding(format!("Failed to send request: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(MemoryError::Embedding(format!(
                "Ollama API error: {} - {}",
                status, text
            )));
        }

        let embedding_response: EmbeddingResponse = response
            .json()
            .await
            .map_err(|e| MemoryError::Embedding(format!("Failed to parse response: {}", e)))?;

        Ok(embedding_response.embedding)
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
}