#[cfg(feature = "llm-local")]
mod tokenizer_tests {
    use arkavo_llm::local::{ModelDownloader, ModelManifest};

    #[tokio::test]
    async fn test_phi2_tokenizer_metadata() {
        // Load manifest
        let manifest = ModelManifest::load().expect("Failed to load manifest");

        // Find phi-2 model
        let phi2_spec = manifest
            .find("phi-2-q4k")
            .expect("phi-2-q4k not found in manifest");

        // Verify base_repo_id is set
        assert!(
            phi2_spec.base_repo_id.is_some(),
            "phi-2-q4k should have base_repo_id set"
        );
        assert_eq!(phi2_spec.base_repo_id.as_ref().unwrap(), "microsoft/phi-2");
        assert_eq!(phi2_spec.tokenizer_type.as_ref().unwrap(), "json");
    }

    #[tokio::test]
    #[ignore] // Ignore by default to avoid downloads in CI
    async fn test_phi2_tokenizer_download() {
        // Load manifest
        let manifest = ModelManifest::load().expect("Failed to load manifest");
        let phi2_spec = manifest
            .find("phi-2-q4k")
            .expect("phi-2-q4k not found in manifest");

        // Create downloader
        let downloader = ModelDownloader::new().expect("Failed to create downloader");

        // Check if model exists in cache (don't download in tests)
        if let Ok(model_path) = downloader.get_model_path(phi2_spec).await {
            println!("Model found at: {}", model_path.display());

            // Check if tokenizer.json exists alongside the model
            let tokenizer_path = model_path.with_file_name("tokenizer.json");
            if tokenizer_path.exists() {
                println!("✓ Tokenizer found at: {}", tokenizer_path.display());
            } else {
                println!("✗ Tokenizer not found at: {}", tokenizer_path.display());
            }
        } else {
            println!("Model not in cache - skipping test to avoid download");
        }
    }
}
