//! Critic post-flight verdict: aggregate cosine similarity vs the baseline plus
//! a tok/s ratio. Embedding is behind a trait so the real ONNX model
//! (arkavo-memory::EmbeddingService) is only required at deploy time; tests use
//! a deterministic fake.

use crate::operator::PromptOutput;
use crate::status::TypedStatus;
use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VerdictError {
    #[error("embedding failed: {0}")]
    Embedding(String),
    #[error("baseline missing output for prompt {0}")]
    MissingBaselineOutput(String),
    #[error("no prompts to compare")]
    NoPrompts,
}

/// Reference outputs + aggregate tok/s captured when a baseline was blessed.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Baseline {
    pub outputs: Vec<BaselineOutput>,
    pub tok_s: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BaselineOutput {
    pub id: String,
    pub text: String,
}

impl Baseline {
    fn output_for(&self, id: &str) -> Option<&str> {
        self.outputs
            .iter()
            .find(|o| o.id == id)
            .map(|o| o.text.as_str())
    }
}

#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, VerdictError>;
}

/// Cosine similarity over the overlapping prefix of two vectors.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Compute the verdict. `acceptance` carries the thresholds from the contract.
pub async fn assess(
    embed: &dyn Embedder,
    outputs: &[PromptOutput],
    baseline: &Baseline,
    min_similarity: f64,
    min_tok_s_ratio: f64,
) -> Result<TypedStatus, VerdictError> {
    if outputs.is_empty() {
        return Err(VerdictError::NoPrompts);
    }
    let mut sim_sum = 0.0f64;
    for o in outputs {
        let base = baseline
            .output_for(&o.id)
            .ok_or_else(|| VerdictError::MissingBaselineOutput(o.id.clone()))?;
        let va = embed.embed(&o.text).await?;
        let vb = embed.embed(base).await?;
        sim_sum += cosine(&va, &vb) as f64;
    }
    let mean_sim = sim_sum / outputs.len() as f64;
    if mean_sim < min_similarity {
        return Ok(TypedStatus::RegressionFailed {
            metric: "similarity".into(),
            value: mean_sim,
            threshold: min_similarity,
        });
    }
    let mean_tok_s = outputs.iter().map(|o| o.tok_s).sum::<f64>() / outputs.len() as f64;
    let ratio = if baseline.tok_s > 0.0 {
        mean_tok_s / baseline.tok_s
    } else {
        1.0
    };
    if ratio < min_tok_s_ratio {
        return Ok(TypedStatus::RegressionFailed {
            metric: "tok_s_ratio".into(),
            value: ratio,
            threshold: min_tok_s_ratio,
        });
    }
    Ok(TypedStatus::Passed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic embedder: a tiny char-frequency vector. Identical text →
    /// identical vector → cosine 1.0; disjoint text → cosine ~0.
    struct FakeEmbedder;

    #[async_trait]
    impl Embedder for FakeEmbedder {
        async fn embed(&self, text: &str) -> Result<Vec<f32>, VerdictError> {
            let mut v = vec![0.0f32; 27];
            for c in text.to_lowercase().chars() {
                if c.is_ascii_lowercase() {
                    v[(c as u8 - b'a') as usize] += 1.0;
                } else {
                    v[26] += 1.0;
                }
            }
            Ok(v)
        }
    }

    fn baseline() -> Baseline {
        Baseline {
            outputs: vec![BaselineOutput {
                id: "p1".into(),
                text: "paris".into(),
            }],
            tok_s: 100.0,
        }
    }

    #[tokio::test]
    async fn identical_output_passes() {
        let outputs = vec![PromptOutput {
            id: "p1".into(),
            text: "paris".into(),
            tok_s: 100.0,
        }];
        let s = assess(&FakeEmbedder, &outputs, &baseline(), 0.87, 0.95)
            .await
            .unwrap();
        assert_eq!(s, TypedStatus::Passed);
    }

    #[tokio::test]
    async fn dissimilar_output_fails_similarity() {
        let outputs = vec![PromptOutput {
            id: "p1".into(),
            text: "zzzzz".into(),
            tok_s: 100.0,
        }];
        match assess(&FakeEmbedder, &outputs, &baseline(), 0.87, 0.95)
            .await
            .unwrap()
        {
            TypedStatus::RegressionFailed { metric, .. } => assert_eq!(metric, "similarity"),
            other => panic!("expected regression, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn slow_output_fails_tok_s() {
        let outputs = vec![PromptOutput {
            id: "p1".into(),
            text: "paris".into(),
            tok_s: 50.0,
        }];
        match assess(&FakeEmbedder, &outputs, &baseline(), 0.87, 0.95)
            .await
            .unwrap()
        {
            TypedStatus::RegressionFailed { metric, .. } => assert_eq!(metric, "tok_s_ratio"),
            other => panic!("expected regression, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_baseline_output_errors() {
        let outputs = vec![PromptOutput {
            id: "other".into(),
            text: "x".into(),
            tok_s: 100.0,
        }];
        assert!(matches!(
            assess(&FakeEmbedder, &outputs, &baseline(), 0.87, 0.95).await,
            Err(VerdictError::MissingBaselineOutput(_))
        ));
    }
}
