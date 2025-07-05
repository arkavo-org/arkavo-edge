#[cfg(feature = "local")]
mod local_smoke_tests {
    use arkavo_llm::local::LocalProvider;
    use arkavo_llm::{Message, Provider, Role};
    use std::path::PathBuf;

    /// Manual smoke test for local development
    ///
    /// This test requires a model file to be present and is intended for
    /// manual testing during development. For CI, use local_ci_test.rs instead.
    ///
    /// Run with: cargo test --features local tinyllama_speaks -- --ignored
    #[tokio::test]
    #[ignore = "requires manual setup - use ci_local_inference_test for CI"]
    async fn tinyllama_speaks() {
        // Check for env var first
        let model_path = if let Ok(path) = std::env::var("TINY_MODEL_PATH") {
            PathBuf::from(path)
        } else {
            // Default path to TinyLlama model
            let default_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("models")
                .join("tinyllama-1b.Q4_K_M.gguf");

            if !default_path.exists() {
                eprintln!(
                    "Skipping test - set TINY_MODEL_PATH env var or place model at: {:?}",
                    default_path
                );
                return;
            }
            default_path
        };

        // Verify the model file exists
        if !model_path.exists() {
            eprintln!("Skipping test - model file not found at: {:?}", model_path);
            return;
        }

        // Create and initialize provider
        let provider = LocalProvider::new(
            "tiny".to_string(),
            Some(model_path.to_string_lossy().into_owned()),
        )
        .unwrap();

        provider.initialize().await.unwrap();

        // Test with a simple prompt
        let messages = vec![Message {
            role: Role::User,
            content: "hello".to_string(),
            images: None,
        }];

        let reply = provider.complete(messages).await.unwrap();

        // Assert we got a non-empty response
        assert!(!reply.trim().is_empty(), "Model returned empty string");
        println!("Model response: {}", reply);
    }
}
