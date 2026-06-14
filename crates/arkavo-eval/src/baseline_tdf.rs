//! Trusted baseline store: TDF-encrypts each baseline and distributes the
//! ciphertext over a content-addressed blob transport (Iroh in production),
//! keyed by commit hash. The small, non-secret pointer — commit, model, BLAKE3
//! digest, and fetch ticket — is persisted to a directory so a restarted or
//! co-located swarm agent can resolve a commit's baseline; only the encrypted
//! content opens, and then only through the TDF policy via a KAS. Generic over
//! the TDF service and transport so tests use the XOR mock + in-memory transport
//! (no KAS) while production injects `OpenTdfService` + `IrohTransport`.

use crate::baseline::{BaselineError, BaselinePointer, BaselineStore};
use crate::digest::{b3_hex, verify_b3};
use crate::verdict::Baseline;
use arkavo_tdf::{
    arkavo_attrs, BlobTransport, Policy, PolicyBuilder, TdfDecryptor, TdfEncryptor, TdfManifest,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Access policy for eval baselines: readable by Arkavo Edge swarm agents. A
/// real KAS enforces this attribute before releasing the key in production.
pub fn eval_baseline_policy() -> Policy {
    PolicyBuilder::new()
        .attribute(arkavo_attrs::ROLE, &["arkavo-edge-agent"])
        .build()
        .expect("static eval-baseline policy has a valid attribute")
}

/// `BaselineStore` that encrypts with `S` and distributes ciphertext via `T`.
pub struct TdfBaselineStore<S, T> {
    tdf: S,
    transport: T,
    policy: Policy,
    dir: PathBuf,
    pointers: Mutex<HashMap<(String, String), BaselinePointer>>,
}

impl<S, T> TdfBaselineStore<S, T> {
    /// Build a store rooted at `dir`. Pointer files already present under `dir`
    /// are loaded so an agent immediately resolves baselines published earlier.
    pub fn new(tdf: S, transport: T, policy: Policy, dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).ok();
        let pointers = Mutex::new(load_pointers(&dir));
        Self {
            tdf,
            transport,
            policy,
            dir,
            pointers,
        }
    }

    fn pointer_path(&self, commit: &str, model: &str) -> PathBuf {
        self.dir.join(pointer_file(commit, model))
    }
}

fn pointer_file(commit: &str, model: &str) -> String {
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
    format!("{}__{}.ptr.json", safe(commit), safe(model))
}

fn load_pointers(dir: &Path) -> HashMap<(String, String), BaselinePointer> {
    let mut map = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return map;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.to_string_lossy().ends_with(".ptr.json") {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(ptr) = serde_json::from_slice::<BaselinePointer>(&bytes) {
                map.insert((ptr.commit.clone(), ptr.model.clone()), ptr);
            }
        }
    }
    map
}

#[async_trait]
impl<S, T> BaselineStore for TdfBaselineStore<S, T>
where
    S: TdfEncryptor + TdfDecryptor + Send + Sync,
    T: BlobTransport,
{
    async fn fetch(&self, commit: &str, model: &str) -> Result<Option<Baseline>, BaselineError> {
        let ptr = self
            .pointers
            .lock()
            .unwrap()
            .get(&(commit.to_string(), model.to_string()))
            .cloned();
        let Some(ptr) = ptr else {
            return Ok(None);
        };

        let manifest_bytes = self
            .transport
            .fetch(&ptr.ticket)
            .await
            .map_err(|e| BaselineError::Backend(format!("transport fetch: {e}")))?;
        // Integrity: distributed content must still hash to the digest recorded
        // at publish time, else the "trusted" baseline has been altered.
        if !verify_b3(&manifest_bytes, &ptr.b3_digest) {
            return Err(BaselineError::Backend(format!(
                "baseline integrity check failed for {commit}/{model}: digest mismatch"
            )));
        }
        let manifest: TdfManifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|e| BaselineError::Backend(format!("manifest decode: {e}")))?;
        let plaintext = self
            .tdf
            .decrypt(&manifest)
            .await
            .map_err(|e| BaselineError::Backend(format!("tdf decrypt: {e}")))?;
        let baseline: Baseline = serde_json::from_slice(&plaintext)
            .map_err(|e| BaselineError::Backend(format!("baseline decode: {e}")))?;
        Ok(Some(baseline))
    }

    async fn publish(
        &self,
        commit: &str,
        model: &str,
        baseline: &Baseline,
    ) -> Result<BaselinePointer, BaselineError> {
        let json = serde_json::to_vec(baseline)
            .map_err(|e| BaselineError::Backend(format!("baseline encode: {e}")))?;
        let manifest = self
            .tdf
            .encrypt(&json, &self.policy)
            .await
            .map_err(|e| BaselineError::Backend(format!("tdf encrypt: {e}")))?;
        let manifest_bytes = serde_json::to_vec(&manifest)
            .map_err(|e| BaselineError::Backend(format!("manifest encode: {e}")))?;
        let b3_digest = b3_hex(&manifest_bytes);
        let ticket = self
            .transport
            .stage(&manifest_bytes)
            .await
            .map_err(|e| BaselineError::Backend(format!("transport stage: {e}")))?;

        let ptr = BaselinePointer {
            commit: commit.to_string(),
            model: model.to_string(),
            b3_digest,
            ticket,
        };
        let bytes = serde_json::to_vec_pretty(&ptr)
            .map_err(|e| BaselineError::Backend(format!("pointer encode: {e}")))?;
        std::fs::write(self.pointer_path(commit, model), &bytes)
            .map_err(|e| BaselineError::Backend(format!("pointer write: {e}")))?;
        self.pointers
            .lock()
            .unwrap()
            .insert((commit.to_string(), model.to_string()), ptr.clone());
        Ok(ptr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verdict::BaselineOutput;
    use arkavo_tdf::testing::{MockBlobTransport, MockTdfService};

    fn sample() -> Baseline {
        Baseline {
            outputs: vec![BaselineOutput {
                id: "p1".into(),
                text: "Canberra".into(),
            }],
            tok_s: 42.0,
        }
    }

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("arkavo-eval-tdf-{tag}-{}", std::process::id()))
    }

    #[tokio::test]
    async fn publish_then_fetch_round_trips_through_tdf_and_transport() {
        let dir = tmpdir("rt");
        std::fs::remove_dir_all(&dir).ok();
        let store = TdfBaselineStore::new(
            MockTdfService::default(),
            MockBlobTransport::new(),
            eval_baseline_policy(),
            &dir,
        );
        assert!(store.fetch("c1", "m").await.unwrap().is_none());
        let b = sample();
        let ptr = store.publish("c1", "m", &b).await.unwrap();
        assert!(ptr.b3_digest.starts_with("b3:"));
        assert!(!ptr.ticket.is_empty());
        assert_eq!(store.fetch("c1", "m").await.unwrap().unwrap(), b);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn staged_blob_is_encrypted_not_plaintext() {
        let dir = tmpdir("enc");
        std::fs::remove_dir_all(&dir).ok();
        let transport = MockBlobTransport::new();
        let store = TdfBaselineStore::new(
            MockTdfService::default(),
            transport.clone(),
            eval_baseline_policy(),
            &dir,
        );
        let ptr = store.publish("c1", "m", &sample()).await.unwrap();
        let staged = transport.fetch(&ptr.ticket).await.unwrap();
        // The answer text must not be stored verbatim — it lives as ciphertext
        // inside the TDF manifest payload.
        assert!(!String::from_utf8_lossy(&staged).contains("Canberra"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn tampered_content_fails_integrity_check() {
        let dir = tmpdir("tamper");
        std::fs::remove_dir_all(&dir).ok();
        let transport = MockBlobTransport::new();
        let store = TdfBaselineStore::new(
            MockTdfService::default(),
            transport.clone(),
            eval_baseline_policy(),
            &dir,
        );
        let ptr = store.publish("c1", "m", &sample()).await.unwrap();
        // Overwrite the staged blob (same ticket, shared Arc store) with garbage.
        let _ = transport
            .clone()
            .with_blob(&ptr.ticket, vec![0xde, 0xad, 0xbe, 0xef]);
        let err = store.fetch("c1", "m").await.unwrap_err();
        assert!(
            err.to_string().contains("integrity"),
            "expected integrity error, got: {err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn pointers_persist_across_instances() {
        let dir = tmpdir("persist");
        std::fs::remove_dir_all(&dir).ok();
        let transport = MockBlobTransport::new();
        let b = sample();
        {
            let store = TdfBaselineStore::new(
                MockTdfService::default(),
                transport.clone(),
                eval_baseline_policy(),
                &dir,
            );
            store.publish("c1", "m", &b).await.unwrap();
        }
        // Fresh store, same dir: the pointer is loaded from disk; the content is
        // fetched from the shared transport and decrypted.
        let store2 = TdfBaselineStore::new(
            MockTdfService::default(),
            transport.clone(),
            eval_baseline_policy(),
            &dir,
        );
        assert_eq!(store2.fetch("c1", "m").await.unwrap().unwrap(), b);
        std::fs::remove_dir_all(&dir).ok();
    }
}
