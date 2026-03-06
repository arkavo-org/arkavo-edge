//! Case-based retrieval for episodes using embedding similarity
//!
//! Indexes episode summaries as embeddings and retrieves similar past episodes
//! for few-shot context injection. Uses a flat cosine similarity search — fast
//! enough for the expected episode count (hundreds, not millions).

use crate::embeddings::EmbeddingService;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

const MAX_INDEXED_EPISODES: usize = 500;

/// Indexed episode entry with its embedding
struct IndexedEpisode {
    episode_id: Uuid,
    summary: String,
    category: String,
    quality_score: f64,
    success: bool,
    embedding: Vec<f32>,
}

/// Case-based retrieval index for episodes
///
/// Stores episode embeddings in memory and retrieves similar past episodes
/// by cosine similarity. Thread-safe via RwLock.
pub struct CaseIndex {
    embedding_service: Arc<EmbeddingService>,
    entries: Arc<RwLock<Vec<IndexedEpisode>>>,
}

impl CaseIndex {
    pub fn new(embedding_service: Arc<EmbeddingService>) -> Self {
        Self {
            embedding_service,
            entries: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Index an episode for future retrieval
    ///
    /// Generates an embedding from the summary text and stores it.
    /// Evicts the oldest entry when the index reaches capacity.
    pub async fn index_episode(
        &self,
        episode_id: Uuid,
        summary: &str,
        category: &str,
        quality_score: f64,
        success: bool,
    ) -> Result<(), String> {
        let embedding = self
            .embedding_service
            .generate_embedding(summary)
            .await
            .map_err(|e| format!("embedding failed: {e}"))?;

        let entry = IndexedEpisode {
            episode_id,
            summary: summary.to_string(),
            category: category.to_string(),
            quality_score,
            success,
            embedding,
        };

        let mut entries = self.entries.write().await;
        if entries.len() >= MAX_INDEXED_EPISODES {
            entries.remove(0);
        }
        entries.push(entry);
        Ok(())
    }

    /// Retrieve similar successful episodes for few-shot context
    ///
    /// Returns (summary, quality_score, similarity) tuples sorted by similarity.
    pub async fn retrieve_similar(
        &self,
        task_description: &str,
        category: Option<&str>,
        limit: usize,
        min_quality: f64,
    ) -> Result<Vec<CaseMatch>, String> {
        let query_embedding = self
            .embedding_service
            .generate_embedding(task_description)
            .await
            .map_err(|e| format!("embedding failed: {e}"))?;

        let entries = self.entries.read().await;
        let mut matches: Vec<CaseMatch> = entries
            .iter()
            .filter(|e| e.success && e.quality_score >= min_quality)
            .filter(|e| category.is_none() || category == Some(e.category.as_str()))
            .map(|e| {
                let similarity =
                    EmbeddingService::cosine_similarity(&query_embedding, &e.embedding);
                CaseMatch {
                    episode_id: e.episode_id,
                    summary: e.summary.clone(),
                    category: e.category.clone(),
                    quality_score: e.quality_score,
                    similarity,
                    success: e.success,
                }
            })
            .filter(|m| m.similarity > 0.3)
            .collect();

        matches.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        matches.truncate(limit);
        Ok(matches)
    }

    /// Retrieve similar failure episodes (anti-examples)
    pub async fn retrieve_failures(
        &self,
        task_description: &str,
        limit: usize,
    ) -> Result<Vec<CaseMatch>, String> {
        let query_embedding = self
            .embedding_service
            .generate_embedding(task_description)
            .await
            .map_err(|e| format!("embedding failed: {e}"))?;

        let entries = self.entries.read().await;
        let mut matches: Vec<CaseMatch> = entries
            .iter()
            .filter(|e| !e.success)
            .map(|e| {
                let similarity =
                    EmbeddingService::cosine_similarity(&query_embedding, &e.embedding);
                CaseMatch {
                    episode_id: e.episode_id,
                    summary: e.summary.clone(),
                    category: e.category.clone(),
                    quality_score: e.quality_score,
                    similarity,
                    success: e.success,
                }
            })
            .filter(|m| m.similarity > 0.3)
            .collect();

        matches.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        matches.truncate(limit);
        Ok(matches)
    }

    /// Format retrieved cases as context for prompt injection
    pub fn format_as_context(successes: &[CaseMatch], failures: &[CaseMatch]) -> String {
        let mut lines = Vec::new();

        if !successes.is_empty() {
            lines.push("Similar successful past episodes:".to_string());
            for case in successes {
                lines.push(format!(
                    "- [quality={:.1}, sim={:.2}] {}",
                    case.quality_score, case.similarity, case.summary
                ));
            }
        }

        if !failures.is_empty() {
            lines.push("Similar past failures (avoid these approaches):".to_string());
            for case in failures {
                lines.push(format!("- [sim={:.2}] {}", case.similarity, case.summary));
            }
        }

        lines.join("\n")
    }

    /// Get the number of indexed episodes
    pub async fn len(&self) -> usize {
        self.entries.read().await.len()
    }

    /// Check if the index is empty
    pub async fn is_empty(&self) -> bool {
        self.entries.read().await.is_empty()
    }
}

/// A matched case from retrieval
#[derive(Debug, Clone)]
pub struct CaseMatch {
    pub episode_id: Uuid,
    pub summary: String,
    pub category: String,
    pub quality_score: f64,
    pub similarity: f32,
    pub success: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_embedding_service() -> Arc<EmbeddingService> {
        Arc::new(EmbeddingService::new())
    }

    #[tokio::test]
    async fn test_case_index_empty() {
        let svc = make_embedding_service();
        let index = CaseIndex::new(svc);
        assert!(index.is_empty().await);
        assert_eq!(index.len().await, 0);
    }

    #[tokio::test]
    async fn test_format_as_context_empty() {
        let ctx = CaseIndex::format_as_context(&[], &[]);
        assert!(ctx.is_empty());
    }

    #[tokio::test]
    async fn test_format_as_context_with_matches() {
        let successes = vec![CaseMatch {
            episode_id: Uuid::new_v4(),
            summary: "Used cooking priority to fix food shortage".to_string(),
            category: "general".to_string(),
            quality_score: 0.8,
            similarity: 0.92,
            success: true,
        }];
        let failures = vec![CaseMatch {
            episode_id: Uuid::new_v4(),
            summary: "Hunted wolves and colonist died".to_string(),
            category: "general".to_string(),
            quality_score: 0.1,
            similarity: 0.85,
            success: false,
        }];

        let ctx = CaseIndex::format_as_context(&successes, &failures);
        assert!(ctx.contains("Similar successful past episodes:"));
        assert!(ctx.contains("cooking priority"));
        assert!(ctx.contains("Similar past failures"));
        assert!(ctx.contains("wolves"));
    }

    #[tokio::test]
    async fn test_eviction_at_capacity() {
        let svc = make_embedding_service();
        let index = CaseIndex::new(svc);

        // Directly populate entries to test eviction logic
        {
            let mut entries = index.entries.write().await;
            for i in 0..MAX_INDEXED_EPISODES {
                entries.push(IndexedEpisode {
                    episode_id: Uuid::new_v4(),
                    summary: format!("episode {i}"),
                    category: "test".to_string(),
                    quality_score: 0.5,
                    success: true,
                    embedding: vec![0.0; 384],
                });
            }
            assert_eq!(entries.len(), MAX_INDEXED_EPISODES);
        }

        // Add one more — should evict oldest
        {
            let mut entries = index.entries.write().await;
            if entries.len() >= MAX_INDEXED_EPISODES {
                entries.remove(0);
            }
            entries.push(IndexedEpisode {
                episode_id: Uuid::new_v4(),
                summary: "newest".to_string(),
                category: "test".to_string(),
                quality_score: 0.5,
                success: true,
                embedding: vec![0.0; 384],
            });
            assert_eq!(entries.len(), MAX_INDEXED_EPISODES);
            assert_eq!(entries.last().unwrap().summary, "newest");
            assert_eq!(entries[0].summary, "episode 1");
        }
    }

    #[tokio::test]
    async fn test_cosine_similarity_filtering() {
        let svc = make_embedding_service();
        let index = CaseIndex::new(svc);

        // Insert entries with known embeddings (orthogonal vectors = 0 similarity)
        {
            let mut entries = index.entries.write().await;

            let mut emb1 = vec![0.0; 384];
            emb1[0] = 1.0;
            entries.push(IndexedEpisode {
                episode_id: Uuid::new_v4(),
                summary: "episode along axis 0".to_string(),
                category: "test".to_string(),
                quality_score: 0.8,
                success: true,
                embedding: emb1,
            });

            let mut emb2 = vec![0.0; 384];
            emb2[1] = 1.0;
            entries.push(IndexedEpisode {
                episode_id: Uuid::new_v4(),
                summary: "episode along axis 1".to_string(),
                category: "test".to_string(),
                quality_score: 0.8,
                success: true,
                embedding: emb2,
            });
        }

        // Query with a vector along axis 0 — should match first, not second
        // (We bypass the embedding service here by checking raw similarity)
        let emb_query = {
            let mut v = vec![0.0; 384];
            v[0] = 1.0;
            v
        };

        let entries = index.entries.read().await;
        let sim0 = EmbeddingService::cosine_similarity(&emb_query, &entries[0].embedding);
        let sim1 = EmbeddingService::cosine_similarity(&emb_query, &entries[1].embedding);

        assert!((sim0 - 1.0).abs() < 0.01, "parallel vectors: {sim0}");
        assert!(sim1.abs() < 0.01, "orthogonal vectors: {sim1}");
    }
}
