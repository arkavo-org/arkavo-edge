use anyhow::Result;
use arkavo_llm::Qwen3Client;
use arkavo_llm::Qwen3Config;
use arkavo_llm::HfTokenizer;

fn main() -> Result<()> {
    // Create a simple test to check tokenizer and model matching
    println!("Testing tokenization and model compatibility...");
    
    // Test tokenizer roundtrip
    let tokenizer = HfTokenizer::new("./crates/arkavo-llm/models/tokenizer.json")?;
    
    // Test a simple message with the expected ChatML format
    let test_message = "<|im_start|>system\nYou are Qwen3, a helpful AI assistant.\n<|im_end|>\n<|im_start|>user\nHello!\n<|im_end|>\n<|im_start|>assistant";
    
    // Encode
    let tokens = tokenizer.encode(test_message)?;
    
    // Print token IDs
    println!("Encoded {} tokens:", tokens.len());
    println!("First 10 tokens: {:?}", &tokens.iter().take(10).collect::<Vec<_>>());
    
    // Check for special tokens
    let has_im_start = tokens.contains(&151644); // <|im_start|>
    let has_im_end = tokens.contains(&151645);   // <|im_end|>
    
    println!("Contains <|im_start|> token (151644): {}", has_im_start);
    println!("Contains <|im_end|> token (151645): {}", has_im_end);
    
    // Decode
    let decoded = tokenizer.decode(&tokens)?;
    
    // Check for roundtrip success
    let roundtrip_success = decoded == test_message;
    println!("Roundtrip success: {}", roundtrip_success);
    
    // Create a test example for the model using a very small token limit
    println!("\nCreating model and testing with a small token limit (32 tokens)...");
    
    // Use the async executor
    futures::executor::block_on(async_main())?;
    
    Ok(())
}

async fn async_main() -> Result<()> {
    // Initialize the client with recommended configuration
    let config = Qwen3Config {
        model_path: String::from("memory://qwen3-0.6b"), // Use embedded model
        temperature: 0.7,                                // Default temperature
        use_gpu: true,                                   // Use GPU if available
        max_tokens: 32,                                  // Short output for testing
    };
    
    let mut client = Qwen3Client::new_with_hf_tokenizer(config);
    println!("Initializing Qwen3 client with explicit HuggingFace tokenizer...");
    client.init().await?;
    
    // Check if model is using GPU
    println!("Using GPU acceleration: {}", client.is_using_gpu());
    println!("Model implementation: {}", client.get_model_impl_name());
    println!("Hardware acceleration: {}", client.get_acceleration_name());
    println!("Tokenizer implementation: {}", client.get_tokenizer_impl_name());
    
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
    println!("Generating response with 32 token limit, please wait...");
    let response = client.generate(prompt).await?;
    
    // Print the response
    println!("\n===== RESPONSE =====");
    println!("{}", response);
    println!("====================");
    
    Ok(())
}