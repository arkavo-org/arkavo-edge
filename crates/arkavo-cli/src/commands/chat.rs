use arkavo_llm::{LlmClient, Message};
use std::env;
use std::io::{self, Write};
use tokio::runtime::Runtime;
use tokio_stream::StreamExt;

pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // Check if there's a --prompt argument
    let prompt = args
        .windows(2)
        .find(|w| w[0] == "--prompt")
        .map(|w| w[1].clone());

    // Create runtime for async operations
    let runtime = Runtime::new()?;

    // Initialize LLM client
    let client = runtime.block_on(async {
        LlmClient::from_env().map_err(|e| format!("Failed to initialize LLM client: {}", e))
    })?;

    println!("Starting chat session...");
    println!("Repository context: {}", get_current_directory());
    println!("LLM Provider: {}", client.provider_name());
    println!("Type 'exit' or 'quit' to end the session.");
    println!();

    // Initialize conversation with system message
    let mut messages = vec![Message::system(
        "You are Arkavo, an AI assistant helping with software development tasks. \
         You have access to the current repository context and can help with code, \
         testing, and development workflows.",
    )];

    // If prompt provided via command line, process it and exit
    if let Some(prompt_text) = prompt {
        messages.push(Message::user(&prompt_text));
        runtime.block_on(process_message(&client, &messages))?;
        return Ok(());
    }

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

        if input == "clear" {
            // Keep only system message
            messages.truncate(1);
            println!("Conversation cleared.");
            continue;
        }

        // Add user message
        messages.push(Message::user(input));

        // Process with LLM
        match runtime.block_on(process_message(&client, &messages)) {
            Ok(response) => {
                messages.push(Message::assistant(&response));
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                // Remove the failed user message
                messages.pop();
            }
        }
    }

    Ok(())
}

async fn process_message(
    client: &LlmClient,
    messages: &[Message],
) -> Result<String, Box<dyn std::error::Error>> {
    print!("Assistant: ");
    io::stdout().flush()?;

    // Use streaming for better UX
    let mut stream = client.stream(messages.to_vec()).await?;
    let mut full_response = String::new();

    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(response) => {
                print!("{}", response.content);
                io::stdout().flush()?;
                full_response.push_str(&response.content);

                if response.done {
                    break;
                }
            }
            Err(e) => {
                return Err(format!("Stream error: {}", e).into());
            }
        }
    }

    println!(); // New line after response
    println!(); // Extra line for readability

    Ok(full_response)
}

fn get_current_directory() -> String {
    match env::current_dir() {
        Ok(path) => path.display().to_string(),
        Err(_) => String::from("Unknown"),
    }
}
