//! Filesystem-backed BaselineStore: persists each baseline as JSON under a
//! directory, keyed by a sanitized `(commit, model)`. Survives restarts.

use crate::baseline::{BaselineError, BaselinePointer, BaselineStore};
use crate::digest::b3_hex;
use crate::verdict::Baseline;
use async_trait::async_trait;
use std::path::PathBuf;

pub struct FileBaselineStore {
    dir: PathBuf,
}

impl FileBaselineStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).ok();
        Self { dir }
    }

    fn key(commit: &str, model: &str) -> String {
        let safe = |s: &str| {
            s.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
        };
        format!("{}__{}.json", safe(commit), safe(model))
    }

    fn path(&self, commit: &str, model: &str) -> PathBuf {
        self.dir.join(Self::key(commit, model))
    }
}

#[async_trait]
impl BaselineStore for FileBaselineStore {
    async fn fetch(&self, commit: &str, model: &str) -> Result<Option<Baseline>, BaselineError> {
        match std::fs::read(self.path(commit, model)) {
            Ok(bytes) => {
                let b: Baseline = serde_json::from_slice(&bytes)
                    .map_err(|e| BaselineError::Backend(e.to_string()))?;
                Ok(Some(b))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(BaselineError::Backend(e.to_string())),
        }
    }

    async fn publish(
        &self,
        commit: &str,
        model: &str,
        baseline: &Baseline,
    ) -> Result<BaselinePointer, BaselineError> {
        let bytes = serde_json::to_vec_pretty(baseline)
            .map_err(|e| BaselineError::Backend(e.to_string()))?;
        std::fs::write(self.path(commit, model), &bytes)
            .map_err(|e| BaselineError::Backend(e.to_string()))?;
        Ok(BaselinePointer {
            commit: commit.into(),
            model: model.into(),
            b3_digest: b3_hex(&bytes),
            ticket: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verdict::BaselineOutput;

    #[tokio::test]
    async fn persists_across_instances() {
        let dir = std::env::temp_dir().join(format!("arkavo-eval-fbs-{}", std::process::id()));
        let b = Baseline {
            outputs: vec![BaselineOutput {
                id: "p1".into(),
                text: "paris".into(),
            }],
            tok_s: 10.0,
        };
        {
            let store = FileBaselineStore::new(&dir);
            assert!(store.fetch("c1", "m").await.unwrap().is_none());
            store.publish("c1", "m", &b).await.unwrap();
        }
        // New instance, same dir — baseline survives.
        let store2 = FileBaselineStore::new(&dir);
        assert_eq!(store2.fetch("c1", "m").await.unwrap().unwrap(), b);
        std::fs::remove_dir_all(dir).ok();
    }
}
