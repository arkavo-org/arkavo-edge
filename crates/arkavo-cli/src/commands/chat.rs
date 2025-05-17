use arkavo_chat::{ChatSession, ChatConfig};
use std::env;

/// Executes the chat command
pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // Set up tokio runtime for async functions
    let rt = tokio::runtime::Runtime::new()?;

    println!("Starting chat session...");
    println!("Repository context: {}", get_current_directory());

    // Create chat config based on args
    let mut config = ChatConfig::default();

    // Check for prompt argument
    if !args.is_empty() {
        if args[0] == "--prompt" || args[0] == "-p" {
            // Single prompt mode with flag
            if args.len() >= 2 {
                config.interactive = false;
                let prompt = &args[1..].join(" ");
                println!("\nProcessing prompt: {}", prompt);

                // Create chat session and process prompt
                let mut session = ChatSession::new(config);
                let response = rt.block_on(session.run_one_shot(prompt))?;
                println!("\n🤖 {}", response);
                return Ok(());
            } else {
                println!("Error: No prompt provided after --prompt flag");
                println!("Usage: arkavo chat --prompt \"Your prompt here\"");
                return Ok(());
            }
        } else if args[0] != "interactive" && args[0] != "--interactive" && args[0] != "-i" {
            // Treat all arguments as the prompt
            config.interactive = false;
            let prompt = &args.join(" ");
            println!("\nProcessing prompt: {}", prompt);

            // Create chat session and process prompt
            let mut session = ChatSession::new(config);
            let response = rt.block_on(session.run_one_shot(prompt))?;
            println!("\n🤖 {}", response);
            return Ok(());
        }
    }

    // Interactive mode
    let mut session = ChatSession::new(config);
    rt.block_on(session.run_interactive())?;

    Ok(())
}

/// Get the current directory path
fn get_current_directory() -> String {
    match env::current_dir() {
        Ok(path) => path.display().to_string(),
        Err(_) => String::from("Unknown"),
    }
}