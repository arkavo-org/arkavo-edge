#[cfg(feature = "local")]
mod local_provider_tests {
    use arkavo_llm::local::LocalProvider;
    use arkavo_llm::{Message, Provider, Role};

    #[tokio::test]
    async fn test_local_provider_creation() {
        // Test creating a local provider without a model path
        let provider = LocalProvider::new("test-model".to_string(), None);
        assert!(provider.is_ok());

        let provider = provider.unwrap();
        assert_eq!(provider.name(), "test-model");
    }

    #[tokio::test]
    async fn test_local_provider_complete_without_init() {
        // Test that complete fails when model is not initialized
        let provider = LocalProvider::new("test-model".to_string(), None).unwrap();

        let messages = vec![Message {
            role: Role::User,
            content: "Hello, world!".to_string(),
            images: None,
        }];

        let result = provider.complete(messages).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Model not loaded"));
    }

    #[tokio::test]
    async fn test_local_provider_stream_without_init() {
        // Test that stream fails when model is not initialized
        let provider = LocalProvider::new("test-model".to_string(), None).unwrap();

        let messages = vec![Message {
            role: Role::User,
            content: "Hello, world!".to_string(),
            images: None,
        }];

        let result = provider.stream(messages).await;
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.to_string().contains("Model not loaded"));
    }
}
