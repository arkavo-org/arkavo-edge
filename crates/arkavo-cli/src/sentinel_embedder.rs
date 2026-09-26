//! Runs the semantic tier's embedding model on llama.cpp, and fetches a
//! pinned embedder from HuggingFace by digest.
//!
//! `arkavo-fingerprint` stays pure Rust (spec: "no C++, no llama dependency
//! in the fingerprint crate"); this is the adapter that plugs a real model
//! into its `Embedder` trait.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use arkavo_fingerprint::{Embedder, EmbedderRecord, EmbeddingPooling};
use arkavo_llama_cpp::LlamaModel;
use arkavo_llama_cpp::embedding::{EmbeddingContext, PoolingType, model_pooling};
use sha2::{Digest, Sha256};

/// A model file plus the context it embeds through.
///
/// # Field order
///
/// `context` is declared before `model` because Rust drops struct fields in
/// declaration order. `EmbeddingContext` borrows the model's raw pointer for
/// its whole lifetime (it is not an owning copy), so the context must be
/// freed before the model it points into — declaring `model` first would
/// free the model while the context's `Drop` still reads it, a
/// use-after-free.
struct Loaded {
    context: EmbeddingContext,
    model: LlamaModel,
}

/// An `Embedder` on llama.cpp.
///
/// The native half sits behind an `Option` so it can be released while the
/// `LlamaEmbedder` itself is still referenced: a provisioned pack's embedder
/// is owned by a set-once, process-lifetime release policy and is never
/// dropped, yet its Metal buffers must be freed before ggml's static
/// destructors run at exit, which abort on buffers still resident.
pub struct LlamaEmbedder {
    loaded: Mutex<Option<Loaded>>,
    digest: String,
    pooling: EmbeddingPooling,
}

impl LlamaEmbedder {
    /// Load a model and build an embedding context for it.
    ///
    /// `pooling` is what the index recorded; refused when the GGUF declares
    /// a different one, so an index built under one pooling can never be
    /// queried under another by accident.
    pub fn load(path: &Path, pooling: EmbeddingPooling) -> Result<Self, String> {
        let model = LlamaModel::from_file(path_str(path)?)?;
        let resolved = resolve_pooling_for(&model, Some(pooling))?;
        let digest = sha256_file(path)?;
        let context = EmbeddingContext::new(&model, to_ffi_pooling(resolved))?;
        Ok(Self {
            loaded: Mutex::new(Some(Loaded { context, model })),
            digest,
            pooling: resolved,
        })
    }

    /// Free the model and context now. Every later `embed` is an error, so a
    /// tier still holding this embedder reports a gap and its gate holds.
    pub fn unload(&self) {
        // Unlike `embed`, a poisoned lock is no reason to refuse: freeing the
        // resources is safe whatever state a panicked decode left them in.
        drop(
            self.loaded
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take(),
        );
    }

    /// Pooling for a build: the GGUF's declared pooling when it has one (and
    /// `flag` must agree with it), else `flag`, else refused.
    pub fn resolve_pooling(
        path: &Path,
        flag: Option<EmbeddingPooling>,
    ) -> Result<EmbeddingPooling, String> {
        let model = LlamaModel::from_file(path_str(path)?)?;
        resolve_pooling_for(&model, flag)
    }
}

impl Embedder for LlamaEmbedder {
    fn digest(&self) -> &str {
        &self.digest
    }

    fn pooling(&self) -> EmbeddingPooling {
        self.pooling
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        // A poisoned lock means some earlier call panicked mid-decode; that
        // context is not trusted to embed the next span, so this is an
        // error rather than a recovery via `into_inner`.
        self.loaded
            .lock()
            .map_err(|_| "embedding context lock poisoned".to_string())?
            .as_mut()
            .map_or_else(
                || Err("the embedder was unloaded".to_string()),
                |loaded| loaded.context.embed(&loaded.model, texts),
            )
    }
}

fn resolve_pooling_for(
    model: &LlamaModel,
    flag: Option<EmbeddingPooling>,
) -> Result<EmbeddingPooling, String> {
    let declared = model_pooling(model).map(from_ffi_pooling);
    match (declared, flag) {
        (Some(declared), Some(flag)) if declared == flag => Ok(declared),
        (Some(declared), Some(flag)) => Err(format!(
            "model declares {declared:?} pooling but {flag:?} was requested"
        )),
        (Some(declared), None) => Ok(declared),
        (None, Some(flag)) => Ok(flag),
        (None, None) => Err("model declares no pooling type; pass one explicitly".to_string()),
    }
}

fn path_str(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("model path is not valid UTF-8: {}", path.display()))
}

/// The one place `EmbeddingPooling` (the fingerprint crate's vocabulary) and
/// `PoolingType` (llama.cpp's vocabulary) meet.
fn to_ffi_pooling(pooling: EmbeddingPooling) -> PoolingType {
    match pooling {
        EmbeddingPooling::Mean => PoolingType::Mean,
        EmbeddingPooling::Cls => PoolingType::Cls,
        EmbeddingPooling::Last => PoolingType::Last,
    }
}

fn from_ffi_pooling(pooling: PoolingType) -> EmbeddingPooling {
    match pooling {
        PoolingType::Mean => EmbeddingPooling::Mean,
        PoolingType::Cls => EmbeddingPooling::Cls,
        PoolingType::Last => EmbeddingPooling::Last,
    }
}

/// Streaming SHA-256 of a file, as lowercase hex.
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("failed to open {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Split `<owner>/<repo>/<file…>` into a HuggingFace repo id and a file path
/// inside it, at the first `/` after the repo. The file segment keeps any
/// further `/` it contains (a file inside a repo subdirectory).
fn split_source(source: &str) -> Result<(String, String), String> {
    let mut parts = source.splitn(3, '/');
    let owner = parts.next().filter(|s| !s.is_empty());
    let repo = parts.next().filter(|s| !s.is_empty());
    let file = parts.next().filter(|s| !s.is_empty());
    match (owner, repo, file) {
        (Some(owner), Some(repo), Some(file)) => Ok((format!("{owner}/{repo}"), file.to_string())),
        _ => Err(format!(
            "embedder source must be <owner>/<repo>/<file>, got {source:?}"
        )),
    }
}

/// The security point of pinning an embedder: refuses a fetched file whose
/// digest does not match what the pack recorded, rather than trusting
/// whatever HuggingFace served this time. Pure and offline so the refusal
/// path can be tested without a download.
fn verify_embedder_digest(fetched: &str, expected: &str) -> Result<(), String> {
    if fetched == expected {
        Ok(())
    } else {
        Err(format!(
            "embedder digest mismatch: expected {expected}, fetched {fetched}"
        ))
    }
}

/// Fetch the embedder named by `record` from HuggingFace.
///
/// Refuses the result if the downloaded bytes do not hash to
/// `record.sha256` (a pinned digest is the whole point: a node must run the
/// exact model the pack's calibration was fitted against).
pub async fn fetch_embedder(record: &EmbedderRecord) -> Result<PathBuf, String> {
    use hf_hub::api::tokio::ApiBuilder;

    let (repo, file) = split_source(&record.source)?;
    let api = ApiBuilder::new()
        .with_progress(true)
        .build()
        .map_err(|e| format!("failed to initialize HuggingFace API: {e}"))?;
    let repo_api = api.repo(hf_hub::Repo::model(repo));
    let path = repo_api
        .get(&file)
        .await
        .map_err(|e| format!("embedder download failed: {e}"))?;

    let fetched = sha256_file(&path)?;
    verify_embedder_digest(&fetched, &record.sha256)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_file_hashes_streamed_bytes() {
        // Bigger than the 64 KiB read buffer, and not a multiple of it, so
        // the loop's multi-chunk path (and the final short read) both run.
        let bytes = vec![0xABu8; 3 * 64 * 1024 + 7];
        let mut path = std::env::temp_dir();
        path.push(format!(
            "arkavo-sentinel-embedder-sha256-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&path, &bytes).expect("write temp file");

        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let expected = hex::encode(hasher.finalize());

        let actual = sha256_file(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(actual.unwrap(), expected);
    }

    /// SEC-adjacent regression: `embed` must never recover a poisoned
    /// context. A model that panicked mid-decode is not trusted to embed the
    /// next span, so the lock's poison must surface as an `Err`, not be
    /// silently cleared via `into_inner`.
    #[test]
    fn a_poisoned_context_lock_is_reported_as_an_error_not_recovered() {
        let Some(path) = std::env::var_os("ARKAVO_TEST_EMBED_MODEL").map(std::path::PathBuf::from)
        else {
            eprintln!("skipping: set ARKAVO_TEST_EMBED_MODEL");
            return;
        };
        let embedder = LlamaEmbedder::load(&path, EmbeddingPooling::Last).unwrap();

        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = embedder.loaded.lock().unwrap();
            panic!("poison the embedding context lock");
        }));
        assert!(poisoned.is_err());

        assert!(embedder.embed(&["still broken"]).is_err());
    }

    #[test]
    fn source_splits_into_repo_and_file() {
        assert_eq!(
            split_source("Qwen/Qwen3-Embedding-0.6B-GGUF/Qwen3-Embedding-0.6B-Q8_0.gguf").unwrap(),
            (
                "Qwen/Qwen3-Embedding-0.6B-GGUF".to_string(),
                "Qwen3-Embedding-0.6B-Q8_0.gguf".to_string()
            )
        );
    }

    #[test]
    fn source_splits_at_the_first_slash_after_the_repo() {
        // A file inside a repo subdirectory keeps its own slashes.
        assert_eq!(
            split_source("owner/repo/sub/dir/file.gguf").unwrap(),
            ("owner/repo".to_string(), "sub/dir/file.gguf".to_string())
        );
    }

    #[test]
    fn a_source_missing_the_file_segment_is_refused() {
        assert!(split_source("owner/repo").is_err());
        assert!(split_source("owner/repo/").is_err());
        assert!(split_source("owner").is_err());
        assert!(split_source("").is_err());
    }

    #[test]
    fn verify_embedder_digest_accepts_a_match() {
        assert!(verify_embedder_digest("abc123", "abc123").is_ok());
    }

    #[test]
    fn verify_embedder_digest_reports_both_hashes_on_mismatch() {
        let err = verify_embedder_digest("fetched-hash", "expected-hash").unwrap_err();
        assert!(err.contains("expected-hash"), "{err}");
        assert!(err.contains("fetched-hash"), "{err}");
    }

    /// The digest check is what makes a fetch trustworthy: a real file
    /// hashed locally, checked against a wrong "expected" value, must be
    /// refused with a message naming both hashes — no network access needed
    /// to exercise the security-relevant path in `fetch_embedder`.
    #[test]
    fn a_locally_hashed_file_is_refused_against_a_wrong_expected_digest() {
        let bytes = b"embedder verification fixture";
        let mut path = std::env::temp_dir();
        path.push(format!(
            "arkavo-sentinel-embedder-digest-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&path, bytes).expect("write temp file");
        let fetched = sha256_file(&path);
        let _ = std::fs::remove_file(&path);
        let fetched = fetched.unwrap();

        let wrong_expected = "0".repeat(64);
        let err = verify_embedder_digest(&fetched, &wrong_expected).unwrap_err();
        assert!(err.contains(&wrong_expected), "{err}");
        assert!(err.contains(&fetched), "{err}");
    }

    #[test]
    fn pooling_round_trips_through_the_ffi_vocabulary() {
        for pooling in [
            EmbeddingPooling::Mean,
            EmbeddingPooling::Cls,
            EmbeddingPooling::Last,
        ] {
            assert_eq!(from_ffi_pooling(to_ffi_pooling(pooling)), pooling);
        }
    }
}
