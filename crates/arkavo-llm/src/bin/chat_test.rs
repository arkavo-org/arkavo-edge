use anyhow::Result;
use arkavo_llm::Qwen3Client;
use arkavo_llm::Qwen3Config;

fn main() -> Result<()> {
    // Use the basic futures executor since we don't have tokio as a dependency yet
    futures::executor::block_on(async_main())
}

async fn async_main() -> Result<()> {
    // Initialize the client with recommended configuration
    let config = Qwen3Config {
        model_path: String::from("memory://qwen3-0.6b"), // Use embedded model
        temperature: 0.7,                                // Default temperature
        use_gpu: true,                                   // Use GPU if available
        max_tokens: 1024,                                // Allow reasonable output length
    };
    
    let mut client = Qwen3Client::new(config);
    println!("Initializing Qwen3 client...");
    client.init().await?;
    
    // Check if model is using GPU
    let is_gpu = client.is_using_gpu();
    println!("Using GPU acceleration: {}", is_gpu);
    println!("Model implementation: {}", client.get_model_impl_name());
    println!("Hardware acceleration: {}", client.get_acceleration_name());
    
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
    println!("Generating response, please wait...");
    let response = client.generate(prompt).await?;
    
    // Print the response
    println!("\n===== RESPONSE =====");
    println!("{}", response);
    println!("====================");
    
    Ok(())
}