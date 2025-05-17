use arkavo_llm::{Qwen3Client, Qwen3Config, extract_response, format_prompt};
use std::env;
use std::io::{self, Write};

/// Executes the chat command
pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // Set up tokio runtime for async functions
    let rt = tokio::runtime::Runtime::new()?;

    println!("Starting chat session...");
    println!("Repository context: {}", get_current_directory());

    // Check for prompt argument
    if !args.is_empty() && args[0] == "prompt" && args.len() >= 2 {
        // Single prompt mode
        let prompt = &args[1..].join(" ");
        println!("\nProcessing prompt: {}", prompt);

        // Run in tokio runtime
        let response = rt.block_on(process_prompt(prompt))?;
        println!("\nResponse: {}", response);
        return Ok(());
    }

    // Initialize LLM client
    let llm_client = rt.block_on(initialize_llm())?;
    println!("\nQwen3-0.6B is ready (local privacy-first LLM)");
    println!("Type 'exit' or 'quit' to end the session\n");

    // Interactive chat loop
    loop {
        print!("> ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        if input == "exit" || input == "quit" {
            println!("Exiting chat session.");
            break;
        }

        // Process with LLM
        match rt.block_on(llm_client.generate(&format_prompt(input))) {
            Ok(response) => {
                let clean_response = extract_response(&response);
                println!("🤖 {}", clean_response);
            }
            Err(e) => {
                println!("Error generating response: {}", e);
            }
        }
    }

    Ok(())
}

/// Initialize the LLM client
async fn initialize_llm() -> Result<Qwen3Client, Box<dyn std::error::Error>> {
    println!("Initializing Qwen3-0.6B LLM...");

    // Create config - use GPU if available
    let config = Qwen3Config {
        model_path: String::from("models/qwen3-0.6b"),
        temperature: 0.7,
        use_gpu: true, // Will fall back to CPU if GPU not available
        max_tokens: 1024,
    };

    let mut client = Qwen3Client::new(config);

    // Check if model is available, download if needed
    if !client.check_model_available().await {
        println!("Model files not found. Downloading Qwen3-0.6B (this might take a while)...");
        client.download_model_if_needed().await?;
    }

    // Initialize model and tokenizer
    client.init().await?;

    Ok(client)
}

/// Process a single prompt and return the response
async fn process_prompt(prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
    // Initialize LLM
    let client = initialize_llm().await?;

    // Generate response
    let full_response = client.generate(&format_prompt(prompt)).await?;
    let response = extract_response(&full_response);

    Ok(response)
}

/// Get the current directory path
fn get_current_directory() -> String {
    match env::current_dir() {
        Ok(path) => path.display().to_string(),
        Err(_) => String::from("Unknown"),
    }
}
