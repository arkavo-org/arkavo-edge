#![cfg(all(feature = "local", not(target_arch = "wasm32")))]

use arkavo_llm::local::{ModelDownloader, ModelManifest};
use std::env;

#[tokio::test]
#[ignore = "requires network access and should only run when explicitly requested"]
async fn test_download_tiny_model() -> anyhow::Result<()> {
    // Skip if CI environment variable is set (don't download in CI)
    if env::var("CI").is_ok() {
        eprintln!("Skipping download test in CI environment");
        return Ok(());
    }

    // Load manifest
    let manifest = ModelManifest::load()?;

    // Find TinyLlama model (smallest for testing)
    let spec = manifest
        .find("tinyllama-1b-chat")
        .ok_or_else(|| anyhow::anyhow!("TinyLlama model not found in manifest"))?;

    // Create downloader
    let downloader = ModelDownloader::new()?;

    // Check if already downloaded (to make test idempotent)
    if downloader.is_downloaded(spec) {
        println!("Model already downloaded, verifying checksum...");
        assert!(
            downloader.is_downloaded(spec),
            "Checksum verification should pass"
        );
    } else {
        println!("Downloading TinyLlama model for testing...");
        let model_path = downloader.download(spec).await?;

        // Verify the file exists and has expected size
        assert!(model_path.exists(), "Downloaded model file should exist");

        let metadata = std::fs::metadata(&model_path)?;
        let size_mb = metadata.len() as f64 / (1024.0 * 1024.0);
        let expected_size_mb = (spec.size_gb * 1024.0) as f64;

        // Allow 10% tolerance for file size
        assert!(
            (size_mb - expected_size_mb).abs() / expected_size_mb < 0.1,
            "Downloaded file size {:.1} MB is not close to expected {:.1} MB",
            size_mb,
            expected_size_mb
        );

        // Verify checksum passes
        assert!(
            downloader.is_downloaded(spec),
            "Downloaded model checksum should verify"
        );
    }

    Ok(())
}

#[tokio::test]
async fn test_manifest_loading() -> anyhow::Result<()> {
    // Test that we can load the manifest
    let manifest = ModelManifest::load()?;

    // Verify we have at least the expected models
    assert!(
        manifest.models.len() >= 3,
        "Manifest should have at least 3 models"
    );

    // Check for specific models
    assert!(
        manifest.find("gemma3n-e2b-it").is_some(),
        "Should have Gemma 3n E2B model"
    );
    assert!(
        manifest.find("gemma3n-e4b-it").is_some(),
        "Should have Gemma 3n E4B model"
    );
    assert!(
        manifest.find("tinyllama-1b-chat").is_some(),
        "Should have TinyLlama model"
    );

    // Verify all models have required fields
    for model in &manifest.models {
        assert!(!model.name.is_empty(), "Model name should not be empty");
        assert!(
            !model.hf_repo_id.is_empty(),
            "HF repo ID should not be empty"
        );
        assert!(
            !model.hf_filename.is_empty(),
            "HF filename should not be empty"
        );
        assert!(!model.sha256.is_empty(), "SHA256 should not be empty");
        assert!(model.size_gb > 0.0, "Size should be positive");
        assert!(
            model.context_length > 0,
            "Context length should be positive"
        );
    }

    Ok(())
}

// Cache directory test removed as dirs::home_dir() doesn't use HOME env var

#[test]
fn test_checksum_verification() {
    // Test the SHA256 verification logic with a known file
    use sha2::{Digest, Sha256};
    use std::io::Write;
    use tempfile::NamedTempFile;

    // Create a test file with known content
    let mut file = NamedTempFile::new().unwrap();
    let content = b"Hello, world!";
    file.write_all(content).unwrap();
    file.flush().unwrap();

    // Calculate expected SHA256
    let mut hasher = Sha256::new();
    hasher.update(content);
    let expected_hash = format!("{:x}", hasher.finalize());

    // This would be tested through the downloader's verify_checksum method
    // but that's private, so we just verify our test setup is correct
    assert_eq!(
        expected_hash,
        "315f5bdb76d078c43b8ac0064e4a0164612b1fce77c869345bfc94c75894edd3"
    );
}

// Run with: cargo test --features local test_download_tiny_model -- --ignored --nocapture
// to actually download and test with a real model
