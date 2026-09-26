#![cfg(feature = "sentinel")]

use std::sync::Arc;

use arkavo_cli::sentinel_embedder::{LlamaEmbedder, sha256_file};
use arkavo_fingerprint::{Embedder, EmbeddingPooling};

fn model_path() -> Option<std::path::PathBuf> {
    std::env::var_os("ARKAVO_TEST_EMBED_MODEL").map(Into::into)
}

#[test]
fn the_digest_is_the_model_file_sha256() {
    let Some(path) = model_path() else {
        eprintln!("skipping: set ARKAVO_TEST_EMBED_MODEL");
        return;
    };
    let embedder = LlamaEmbedder::load(&path, EmbeddingPooling::Last).unwrap();
    assert_eq!(embedder.digest(), sha256_file(&path).unwrap());
}

#[test]
fn concurrent_embeds_serialise_and_agree() {
    let Some(path) = model_path() else {
        eprintln!("skipping: set ARKAVO_TEST_EMBED_MODEL");
        return;
    };
    let embedder = Arc::new(LlamaEmbedder::load(&path, EmbeddingPooling::Last).unwrap());
    let reference = embedder.embed(&["the quarterly board minutes"]).unwrap();
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let e = embedder.clone();
            std::thread::spawn(move || e.embed(&["the quarterly board minutes"]).unwrap())
        })
        .collect();
    for h in handles {
        let v = h.join().unwrap();
        let dot: f32 = v[0].iter().zip(&reference[0]).map(|(a, b)| a * b).sum();
        let n: f32 = reference[0].iter().map(|x| x * x).sum();
        assert!((dot / n - 1.0).abs() < 1e-3);
    }
}

/// stories15M declares no pooling type in its GGUF metadata, so a build
/// without a `flag` is refused and one with a flag adopts it. Qwen3-Embedding
/// declares `Last` pooling, so a build without a flag adopts the declared
/// value and a flag that disagrees with it is refused instead. Which branch
/// applies is read from `resolve_pooling` itself rather than hardcoded, so
/// the test is correct for either model named by `ARKAVO_TEST_EMBED_MODEL`.
#[test]
fn a_build_without_declared_or_given_pooling_is_refused() {
    let Some(path) = model_path() else {
        eprintln!("skipping: set ARKAVO_TEST_EMBED_MODEL");
        return;
    };
    match LlamaEmbedder::resolve_pooling(&path, None) {
        Err(_) => {
            // No declared pooling (e.g. stories15M): an explicit flag is required
            // and is honoured.
            assert_eq!(
                LlamaEmbedder::resolve_pooling(&path, Some(EmbeddingPooling::Last)).unwrap(),
                EmbeddingPooling::Last
            );
        }
        Ok(declared) => {
            // A declared pooling (e.g. Qwen3-Embedding) is adopted when no flag
            // is given, matched when the flag agrees, and refused when it does
            // not.
            eprintln!("model at {} declares {declared:?} pooling", path.display());
            assert_eq!(
                LlamaEmbedder::resolve_pooling(&path, Some(declared)).unwrap(),
                declared
            );
            let conflicting = if declared == EmbeddingPooling::Last {
                EmbeddingPooling::Mean
            } else {
                EmbeddingPooling::Last
            };
            assert!(LlamaEmbedder::resolve_pooling(&path, Some(conflicting)).is_err());
        }
    }
}

/// Regression: `arkavo chat --pack` aborted at every exit on Metal
/// (`ggml_metal_device_free` asserting on live residency sets), because the
/// provisioned embedder lives inside the set-once release policy and was
/// never dropped. Unloading frees its llama.cpp resources in place; an
/// embed after that is an error — which the semantic tier reports as
/// `Unavailable`, a gap, so the gate holds rather than releasing unjudged.
#[test]
fn an_unloaded_embedder_refuses_to_embed() {
    let Some(path) = model_path() else {
        eprintln!("skipping: set ARKAVO_TEST_EMBED_MODEL");
        return;
    };
    let embedder = LlamaEmbedder::load(&path, EmbeddingPooling::Last).unwrap();
    assert!(embedder.embed(&["the quarterly board minutes"]).is_ok());

    embedder.unload();
    embedder.unload();

    let err = embedder
        .embed(&["the quarterly board minutes"])
        .unwrap_err();
    assert!(err.contains("unloaded"), "{err}");
}

#[test]
fn dropping_the_release_guard_unloads_the_provisioned_embedder() {
    let Some(path) = model_path() else {
        eprintln!("skipping: set ARKAVO_TEST_EMBED_MODEL");
        return;
    };
    let embedder = Arc::new(LlamaEmbedder::load(&path, EmbeddingPooling::Last).unwrap());
    // The policy keeps its own reference for the rest of the process; the
    // guard must release the native resources regardless.
    let held_by_policy: Arc<dyn Embedder> = embedder.clone();

    drop(arkavo_cli::commands::chat_pack::NativeRelease::new(Some(
        embedder,
    )));

    assert!(held_by_policy.embed(&["after the session"]).is_err());
}
