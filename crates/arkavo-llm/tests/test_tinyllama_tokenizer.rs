#![cfg(all(feature = "local", not(target_arch = "wasm32")))]

use arkavo_llm::local::{LocalProvider, ModelDownloader, ModelManifest};
use arkavo_llm::{Message, Provider, Role};

/// Test that TinyLlama v1.0 from TheBloke has embedded tokenizer
#[tokio::test]
async fn test_tinyllama_has_embedded_tokenizer() -> anyhow::Result<()> {
    // Load manifest and find TinyLlama
    let manifest = ModelManifest::load()?;
    let spec = manifest
        .find("tinyllama-1b-chat")
        .ok_or_else(|| anyhow::anyhow!("tinyllama-1b-chat not found in manifest"))?;

    // Ensure model is downloaded
    let downloader = ModelDownloader::new()?;
    let model_path = downloader.get_model_path(spec).await?;

    // Create and initialize provider
    let provider = LocalProvider::new(
        "tinyllama-1b-chat".to_string(),
        Some(model_path.to_string_lossy().into_owned()),
    )?;

    // Initialize should succeed if tokenizer is embedded
    provider.initialize().await?;
    eprintln!("✓ TinyLlama initialized successfully with embedded tokenizer");

    // Quick inference test
    let messages = vec![Message {
        role: Role::User,
        content: "Say hello.".to_string(),
        images: None,
    }];

    let reply = provider.complete(messages).await?;
    assert!(
        !reply.trim().is_empty(),
        "Model should return non-empty response"
    );
    eprintln!("✓ Model response: {}", reply.trim());

    Ok(())
}
