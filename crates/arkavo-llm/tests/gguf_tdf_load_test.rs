//! `.gguf.tdf` detection and fail-closed behaviour in the provider.

#![cfg(feature = "llama-cpp")]

use arkavo_llm::gguf_tdf::is_protected_model_path;
use arkavo_llm::{Error, LlamaCppProvider, SamplingConfig};

/// `expect_err` needs `Debug` on the success type, which the provider does
/// not implement.
fn expect_err<T>(result: Result<T, Error>, what: &str) -> Error {
    match result {
        Ok(_) => panic!("{what}"),
        Err(err) => err,
    }
}

#[test]
fn the_sync_constructor_refuses_a_protected_model() {
    // Without a payload key there is no way to read the weights, and the
    // profile forbids falling back to a sibling plaintext model.
    let err = expect_err(
        LlamaCppProvider::new_with_config(
            "protected".to_string(),
            "/tmp/does-not-exist/model.gguf.tdf".to_string(),
            None,
            SamplingConfig::default(),
        ),
        "a protected model must not load without a key",
    );

    let msg = err.to_string();
    assert!(msg.contains("GGUFTDF_KAS_DENIED"), "got: {msg}");
    assert!(
        msg.contains("new_protected"),
        "error should name the way in: {msg}"
    );
}

#[test]
fn a_protected_model_never_falls_back_to_a_sibling_plaintext() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("model.gguf");
    let protected = dir.path().join("model.gguf.tdf");
    // A real GGUF next to the protected artifact; it must never be loaded.
    std::fs::write(&plain, b"GGUF\x03\x00\x00\x00").unwrap();
    std::fs::write(&protected, b"PK\x03\x04not-a-real-archive").unwrap();

    let err = expect_err(
        LlamaCppProvider::new_with_config(
            "protected".to_string(),
            protected.to_string_lossy().to_string(),
            None,
            SamplingConfig::default(),
        ),
        "must fail closed",
    );
    assert!(err.to_string().contains("GGUFTDF_KAS_DENIED"));

    // Loading with a key still fails, because the archive is not a real one,
    // and it fails on the archive rather than silently reading the sibling.
    let err = expect_err(
        LlamaCppProvider::new_protected(
            "protected".to_string(),
            protected.to_string_lossy().to_string(),
            SamplingConfig::default(),
            [0u8; 32],
        ),
        "a corrupt archive must not load",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("Failed to open") || msg.contains("GGUFTDF"),
        "should report the archive, not the sibling: {msg}"
    );
}

#[test]
fn plain_gguf_paths_are_not_treated_as_protected() {
    assert!(!is_protected_model_path("/models/gemma.gguf"));
    assert!(is_protected_model_path("/models/gemma.gguf.tdf"));
}
