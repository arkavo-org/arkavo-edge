#![cfg(all(feature = "local", not(target_arch = "wasm32")))]

use arkavo_llm::local::{LocalProvider, ModelDownloader, ModelManifest};
use arkavo_llm::{Message, Provider, Role};

/// CI smoke test that downloads gemma3n-e2b-it if needed and runs inference
///
/// This test is designed to run in CI environments and will:
/// 1. Download gemma3n-e2b-it model if not already cached
/// 2. Verify the download with SHA-256
/// 3. Run a simple inference test
///
/// Note: Many GGUF models from unsloth don't include embedded tokenizers,
/// so this test will gracefully skip if tokenizer loading fails.
/// Set CI_SKIP_LOCAL_TEST=1 to skip this test in CI
#[tokio::test]
async fn ci_local_inference_test() -> anyhow::Result<()> {
    // Allow skipping in CI if needed
    if std::env::var("CI_SKIP_LOCAL_TEST").is_ok() {
        eprintln!("Skipping local inference test due to CI_SKIP_LOCAL_TEST");
        return Ok(());
    }

    // Load manifest and find gemma3n-e2b-it
    let manifest = ModelManifest::load()?;
    let spec = manifest
        .find("gemma3n-e2b-it")
        .ok_or_else(|| anyhow::anyhow!("gemma3n-e2b-it not found in manifest"))?;

    // Create downloader and ensure model is available
    let downloader = ModelDownloader::new()?;
    let model_path = if downloader.is_downloaded(spec) {
        eprintln!("Using cached gemma3n-e2b-it model");
        downloader.get_model_path(spec)
    } else {
        eprintln!("Downloading gemma3n-e2b-it model for CI test...");
        downloader.download(spec).await?
    };

    // Verify model file exists
    assert!(
        model_path.exists(),
        "Model file should exist at {:?}",
        model_path
    );

    // Create and initialize provider
    let provider = LocalProvider::new(
        "gemma3n-e2b-it".to_string(),
        Some(model_path.to_string_lossy().into_owned()),
    )?;

    // Try to initialize - if tokenizer is missing, skip test
    match provider.initialize().await {
        Ok(()) => {
            eprintln!("Provider initialized successfully");
        }
        Err(e) if e.to_string().contains("Tokenizer not loaded") => {
            eprintln!("Skipping test due to missing tokenizer: {}", e);
            eprintln!(
                "This is expected for some GGUF models that don't include embedded tokenizers"
            );
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    }

    // Test with a simple prompt
    let messages = vec![Message {
        role: Role::User,
        content: "Say hello in one word.".to_string(),
        images: None,
    }];

    let start = std::time::Instant::now();
    let reply = provider.complete(messages).await?;
    let duration = start.elapsed();

    // Verify we got a response
    assert!(
        !reply.trim().is_empty(),
        "Model should return non-empty response"
    );

    // Log performance metrics
    eprintln!("Model response: {}", reply.trim());
    eprintln!("Inference time: {:?}", duration);

    // Basic sanity check - response should be reasonably short for "say hello in one word"
    assert!(
        reply.len() < 500,
        "Response suspiciously long for simple prompt: {} chars",
        reply.len()
    );

    Ok(())
}

/// Test that model manifest is valid and contains expected models
#[test]
fn test_manifest_validity() -> anyhow::Result<()> {
    let manifest = ModelManifest::load()?;

    // Verify gemma3n-e2b-it is present (required for CI)
    assert!(
        manifest.find("gemma3n-e2b-it").is_some(),
        "gemma3n-e2b-it must be in manifest for CI tests"
    );

    // Verify all models have valid checksums (64 char hex)
    for model in &manifest.models {
        assert_eq!(
            model.sha256.len(),
            64,
            "Model {} has invalid SHA256 length",
            model.name
        );
        assert!(
            model.sha256.chars().all(|c| c.is_ascii_hexdigit()),
            "Model {} has invalid SHA256 characters",
            model.name
        );
    }

    Ok(())
}
