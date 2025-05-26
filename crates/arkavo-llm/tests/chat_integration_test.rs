use anyhow::Result;
use arkavo_llm::{Qwen3Client, Qwen3Config};

// Skip this test by default since it runs a full model inference which is slow
#[test]
#[ignore]
fn test_chat_generation() -> Result<()> {
    // Use the basic futures executor since we don't have tokio as a dependency yet
    futures::executor::block_on(async {
        // Initialize the client with recommended configuration
        let config = Qwen3Config {
            model_path: String::from("memory://qwen3-0.6b"), // Use embedded model
            temperature: 0.7,                                // Default temperature
            use_gpu: true,                                   // Use GPU if available
            max_tokens: 1024,                                // Allow reasonable output length
        };
        
        let mut client = Qwen3Client::new(config);
        client.init().await?;
        
        // Format a chat prompt in Qwen3 format
        let prompt = "<|im_start|>system
You are Qwen3, a helpful AI assistant created by Arkavo Edge.
<|im_end|>
<|im_start|>user
Explain how to resolve a merge conflict in git.
<|im_end|>
<|im_start|>assistant
";
        
        // Generate a response
        let response = client.generate(prompt).await?;
        
        // Check the response contains relevant keywords for the question
        let contains_git = response.to_lowercase().contains("git");
        let contains_merge = response.to_lowercase().contains("merge") || 
                            response.to_lowercase().contains("conflict");
        
        assert!(contains_git, "Response should mention git");
        assert!(contains_merge, "Response should mention merge or conflict");
        
        Ok(())
    })
}