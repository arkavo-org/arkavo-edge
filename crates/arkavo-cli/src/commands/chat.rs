use std::env;
use std::io::{self, Write};
use arkavo_llm::{LlmClient, ChatRequest};

pub async fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // Check for --print flag
    let print_mode = args.contains(&"--print".to_string()) || args.contains(&"-p".to_string());
    
    // Parse prompt if in print mode
    if print_mode {
        let prompt_start = args.iter().position(|arg| !arg.starts_with("--") && !arg.starts_with("-"))
            .unwrap_or(args.len());
        
        if prompt_start >= args.len() {
            eprintln!("Error: No prompt provided for --print mode");
            return Err("No prompt provided".into());
        }
        
        let prompt = args[prompt_start..].join(" ");
        return execute_print_mode(&prompt).await;
    }
    
    // Interactive mode
    if !print_mode {
        println!("Starting chat session...");
        println!("Repository context: {}", get_current_directory());
        println!("Connecting to LLM...");
    }
    
    // Initialize LLM client
    let client = match LlmClient::from_env_with_discovery().await {
        Ok(client) => {
            if !print_mode {
                println!("✓ Connected to {} provider", client.provider_name());
            }
            client
        }
        Err(e) => {
            if print_mode {
                eprintln!("Error: Failed to initialize LLM client: {}", e);
            } else {
                println!("✗ Failed to initialize LLM client: {}", e);
                println!("Make sure Ollama is running: ollama serve");
            }
            return Err(e.into());
        }
    };
    
    if !print_mode {
        println!("Type 'exit' or 'quit' to end the session.\n");
    }

    // Interactive mode loop
    if !print_mode {
        execute_interactive_mode(&client).await?;
    }

    Ok(())
}

async fn execute_interactive_mode(client: &LlmClient) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        print!("> ");
        io::stdout().flush()?;

        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(0) => break, // EOF
            Ok(_) => {},
            Err(e) => {
                println!("Error reading input: {}", e);
                break;
            }
        }

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        if input == "exit" || input == "quit" {
            println!("Exiting chat session.");
            break;
        }

        // Send to LLM with coding context
        let request = ChatRequest::new(format!(
            "You are an expert coding assistant working on the Arkavo Edge project. \
             Repository context: {}. User query: {}",
            get_current_directory(),
            input
        ));

        print!("Agent: ");
        io::stdout().flush()?;

        match client.chat_unified(request).await {
            Ok(response) => {
                println!("{}\n", response);
            }
            Err(e) => {
                println!("Error: {}\n", e);
            }
        }
    }

    Ok(())
}

async fn execute_print_mode(prompt: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = LlmClient::from_env_with_discovery().await?;
    
    let request = ChatRequest::new(format!(
        "You are an expert coding assistant working on the Arkavo Edge project. \
         Repository context: {}. User query: {}",
        get_current_directory(),
        prompt
    ));

    match client.chat_unified(request).await {
        Ok(response) => {
            println!("{}", response);
            Ok(())
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            Err(e.into())
        }
    }
}

fn get_current_directory() -> String {
    match env::current_dir() {
        Ok(path) => path.display().to_string(),
        Err(_) => String::from("Unknown"),
    }
}
