//! Historian role: stores/retrieves baselines. The trait is backend-agnostic;
//! the TDF+iroh implementation lands in Part 2. `MemBaselineStore` is used by
//! tests and the one-shot CLI demo.

use crate::verdict::Baseline;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BaselineError {
    #[error("baseline backend error: {0}")]
    Backend(String),
}

/// A shareable pointer to a published baseline.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BaselinePointer {
    pub commit: String,
    pub model: String,
    /// `b3:<hex>` content address of the (encrypted) baseline artifact.
    pub b3_digest: String,
    /// Fetch handle (iroh ticket string in the real impl; empty for in-memory).
    pub ticket: String,
}

#[async_trait]
pub trait BaselineStore: Send + Sync {
    /// Fetch the baseline blessed at `commit` for `model`, if any.
    async fn fetch(&self, commit: &str, model: &str) -> Result<Option<Baseline>, BaselineError>;
    /// Publish `baseline` as the trusted baseline for `commit`/`model`.
    async fn publish(
        &self,
        commit: &str,
        model: &str,
        baseline: &Baseline,
    ) -> Result<BaselinePointer, BaselineError>;
}

/// In-memory store keyed by `(commit, model)`.
#[derive(Default)]
pub struct MemBaselineStore {
    inner: Mutex<HashMap<(String, String), Baseline>>,
}

impl MemBaselineStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl BaselineStore for MemBaselineStore {
    async fn fetch(&self, commit: &str, model: &str) -> Result<Option<Baseline>, BaselineError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .get(&(commit.to_string(), model.to_string()))
            .cloned())
    }

    async fn publish(
        &self,
        commit: &str,
        model: &str,
        baseline: &Baseline,
    ) -> Result<BaselinePointer, BaselineError> {
        let bytes =
            serde_json::to_vec(baseline).map_err(|e| BaselineError::Backend(e.to_string()))?;
        let digest = crate::digest::b3_hex(&bytes);
        self.inner
            .lock()
            .unwrap()
            .insert((commit.to_string(), model.to_string()), baseline.clone());
        Ok(BaselinePointer {
            commit: commit.into(),
            model: model.into(),
            b3_digest: digest,
            ticket: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verdict::BaselineOutput;

    #[tokio::test]
    async fn publish_then_fetch_round_trips() {
        let store = MemBaselineStore::new();
        assert!(store.fetch("c1", "m").await.unwrap().is_none());
        let b = Baseline {
            outputs: vec![BaselineOutput {
                id: "p1".into(),
                text: "paris".into(),
            }],
            tok_s: 100.0,
        };
        let ptr = store.publish("c1", "m", &b).await.unwrap();
        assert!(ptr.b3_digest.starts_with("b3:"));
        assert_eq!(store.fetch("c1", "m").await.unwrap().unwrap(), b);
    }
}
