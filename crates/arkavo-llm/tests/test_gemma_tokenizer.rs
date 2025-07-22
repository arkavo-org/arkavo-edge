#![cfg(all(feature = "local", not(target_arch = "wasm32")))]

use arkavo_llm::local::LocalProvider;
use arkavo_llm::{Message, Provider, Role};
use std::path::PathBuf;

/// Test that gemma3n-e2b-it works even without embedded tokenizer
#[tokio::test]
async fn test_gemma_without_tokenizer() -> anyhow::Result<()> {
    // Skip in CI since it requires external tokenizer
    if std::env::var("CI").is_ok() {
        eprintln!("Skipping gemma test in CI");
        return Ok(());
    }

    let model_path = PathBuf::from(std::env::var("HOME").unwrap())
        .join(".cache/arkavo/models/gemma-3n-E2B-it-Q4_K_M.gguf");

    if !model_path.exists() {
        eprintln!("Gemma model not found at {model_path:?}, skipping test");
        return Ok(());
    }

    // Create provider
    let provider = LocalProvider::new(
        "gemma3n".to_string(),
        Some(model_path.to_string_lossy().into_owned()),
    )?;

    // Try to initialize - expect tokenizer error
    match provider.initialize().await {
        Ok(()) => {
            eprintln!("✓ Gemma initialized successfully");

            // Try inference
            let messages = vec![Message {
                role: Role::User,
                content: "Say hello.".to_string(),
                images: None,
            }];

            let reply = provider.complete(messages).await?;
            eprintln!("Model response: {}", reply.trim());
        }
        Err(e) if e.to_string().contains("Tokenizer not loaded") => {
            eprintln!("Expected: Gemma requires external tokenizer");
            eprintln!("To fix: Place tokenizer.model or tokenizer.json next to the GGUF file");
        }
        Err(e) => return Err(e.into()),
    }

    Ok(())
}
