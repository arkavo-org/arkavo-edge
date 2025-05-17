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
    if !args.is_empty() {
        if args[0] == "--prompt" || args[0] == "-p" {
            // Single prompt mode with flag
            if args.len() >= 2 {
                let prompt = &args[1..].join(" ");
                println!("\nProcessing prompt: {}", prompt);

                // Show processing indicator
                print!("Thinking");
                io::stdout().flush()?;
                for _ in 0..3 {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    print!(".");
                    io::stdout().flush()?;
                }
                println!();

                // Run in tokio runtime
                let response = rt.block_on(process_prompt(prompt))?;
                println!("\n🤖 {}", response);
                return Ok(());
            } else {
                println!("Error: No prompt provided after --prompt flag");
                println!("Usage: arkavo chat --prompt \"Your prompt here\"");
                return Ok(());
            }
        } else if args[0] != "interactive" && args[0] != "--interactive" && args[0] != "-i" {
            // Treat all arguments as the prompt
            let prompt = &args.join(" ");
            println!("\nProcessing prompt: {}", prompt);

            // Show processing indicator
            print!("Thinking");
            io::stdout().flush()?;
            for _ in 0..3 {
                std::thread::sleep(std::time::Duration::from_millis(300));
                print!(".");
                io::stdout().flush()?;
            }
            println!();

            // Run in tokio runtime
            let response = rt.block_on(process_prompt(prompt))?;
            println!("\n🤖 {}", response);
            return Ok(());
        }
    }

    // Initialize LLM client
    let (llm_client, temp_path) = rt.block_on(initialize_llm())?;
    println!("\n🤖 Qwen3-0.6B is ready (local privacy-first LLM)");
    println!("🔐 All processing happens on your device - no data is sent to external services");
    println!("💬 Type 'exit' or 'quit' to end the session");
    println!("❓ Try asking 'What can you help me with?' to learn more\n");

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
            println!("Exiting chat session. Thank you for using Arkavo Edge!");
            
            // Clean up temporary files
            if let Err(e) = cleanup_temp_files(&temp_path) {
                eprintln!("Warning: Error cleaning up temporary files: {}", e);
            }
            
            break;
        }

        // Show "thinking" animation
        let thinking_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thinking_stop_clone = thinking_stop.clone();
        
        let thinking_thread = std::thread::spawn(move || {
            let thinking_chars = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
            let mut i = 0;
            while !thinking_stop_clone.load(std::sync::atomic::Ordering::Relaxed) {
                print!("\r🧠 {}", thinking_chars[i]);
                io::stdout().flush().unwrap();
                i = (i + 1) % thinking_chars.len();
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });

        // Process with LLM
        let result = rt.block_on(llm_client.generate(&format_prompt(input)));
        
        // Stop thinking animation
        thinking_stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if thinking_thread.join().is_err() {
            // Ignore any errors from the thinking thread
        }
        print!("\r                      \r"); // Clear the thinking animation
        io::stdout().flush()?;

        // Process response
        match result {
            Ok(response) => {
                let clean_response = extract_response(&response);
                println!("🤖 {}", clean_response);
            }
            Err(e) => {
                println!("❌ Error generating response: {}", e);
                println!("Please try a different query or check if the model files are correctly installed.");
            }
        }
        
        println!(); // Add a line break for better readability
    }

    Ok(())
}

/// Initialize the LLM client
async fn initialize_llm() -> Result<(Qwen3Client, String), Box<dyn std::error::Error>> {
    println!("Initializing Qwen3-0.6B LLM...");

    // Create unique temporary directory for this session
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    
    let model_path = std::env::temp_dir()
        .join(format!("arkavo-tmp-model-{}", timestamp))
        .to_string_lossy()
        .to_string();
    
    let config = Qwen3Config {
        model_path: model_path.clone(),  // Use temporary location for minimal disk usage
        temperature: 0.7,
        use_gpu: true,
        max_tokens: 1024,
    };

    let mut client = Qwen3Client::new(config);

    // Initialize model and tokenizer
    client.init().await?;

    Ok((client, model_path))
}

/// Clean up temporary files after use
fn cleanup_temp_files(temp_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::Path::new(temp_path);
    if path.exists() {
        // Remove each file individually first
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            if entry.path().is_file() {
                std::fs::remove_file(entry.path())?;
            }
        }
        // Then remove the directory
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}

/// Process a single prompt and return the response
async fn process_prompt(prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
    // Initialize LLM (initialization is done inside this function)
    let (client, temp_path) = initialize_llm().await?;

    // Generate response (client is already initialized)
    let full_response = client.generate(&format_prompt(prompt)).await?;
    let response = extract_response(&full_response);
    
    // Clean up temporary files
    if let Err(e) = cleanup_temp_files(&temp_path) {
        eprintln!("Warning: Error cleaning up temporary files: {}", e);
    }

    Ok(response)
}

/// Get the current directory path
fn get_current_directory() -> String {
    match env::current_dir() {
        Ok(path) => path.display().to_string(),
        Err(_) => String::from("Unknown"),
    }
}
