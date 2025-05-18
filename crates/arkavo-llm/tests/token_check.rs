use anyhow::Result;
use arkavo_llm::{HfTokenizer, Qwen3Client, Qwen3Config};

#[test]
fn test_tokenizer_compatibility() -> Result<()> {
    // Create a simple test to check tokenizer and model matching
    // Try different paths since tests run from different working directory
    let possible_paths = [
        "../models/tokenizer.json",
        "models/tokenizer.json",
        "./crates/arkavo-llm/models/tokenizer.json",
    ];
    
    let mut tokenizer = None;
    for path in possible_paths {
        if let Ok(t) = HfTokenizer::new(path) {
            tokenizer = Some(t);
            break;
        }
    }
    
    let tokenizer = tokenizer.ok_or_else(|| anyhow::anyhow!("Failed to load tokenizer from any path"))?;
    
    // Test a simple message with the expected ChatML format
    let test_message = "<|im_start|>system\nYou are Qwen3, a helpful AI assistant.\n<|im_end|>\n<|im_start|>user\nHello!\n<|im_end|>\n<|im_start|>assistant";
    
    // Encode
    let tokens = tokenizer.encode(test_message)?;
    
    // Check for special tokens
    let has_im_start = tokens.contains(&151644); // <|im_start|>
    let has_im_end = tokens.contains(&151645);   // <|im_end|>
    
    assert!(has_im_start, "Missing <|im_start|> token (151644)");
    assert!(has_im_end, "Missing <|im_end|> token (151645)");
    
    // Decode
    let decoded = tokenizer.decode(&tokens)?;
    
    // Check for roundtrip success
    assert_eq!(decoded, test_message, "Tokenizer roundtrip failed");
    
    Ok(())
}

// Skip model test by default since it's slow and resource-intensive
// Use #[ignore] attribute to only run it when explicitly requested
#[test]
#[ignore]
fn test_model_with_tokenizer() -> Result<()> {
    // Use the async executor
    futures::executor::block_on(async {
        // Initialize the client with test configuration
        let config = Qwen3Config {
            model_path: String::from("memory://qwen3-0.6b"), // Use embedded model
            temperature: 0.7,                                // Default temperature
            use_gpu: true,                                   // Use GPU if available
            max_tokens: 32,                                  // Short output for testing
        };
        
        let mut client = Qwen3Client::new_with_hf_tokenizer(config);
        client.init().await?;
        
        // Format a chat prompt in Qwen3 format with a simple request
        let prompt = "<|im_start|>system
You are Qwen3, a helpful AI assistant.
<|im_end|>
<|im_start|>user
Write a short 'hello world' program in Lua.
<|im_end|>
<|im_start|>assistant
";
        
        // Generate a response
        let response = client.generate(prompt).await?;
        
        // Check that we got a non-empty response
        assert!(!response.trim().is_empty(), "Model returned empty response");
        
        Ok(())
    })
}