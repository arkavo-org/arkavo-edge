#[cfg(feature = "local")]
mod local_smoke_tests {
    use arkavo_llm::local::LocalProvider;
    use arkavo_llm::{Message, Provider, Role};
    use std::path::PathBuf;

    #[tokio::test]
    #[ignore] // Remove this when we have the model file
    async fn tinyllama_speaks() {
        // Path to TinyLlama model - this would be committed via Git LFS
        let gguf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("models")
            .join("tinyllama-1b.Q4_K_M.gguf");

        // Skip test if model file doesn't exist
        if !gguf.exists() {
            eprintln!("Skipping test - model file not found at: {:?}", gguf);
            return;
        }

        // Create and initialize provider
        let provider = LocalProvider::new(
            "tiny".to_string(),
            Some(gguf.to_string_lossy().into_owned()),
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
