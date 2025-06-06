use arkavo_llm::{LlmClient, Message};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
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
    println!("Commands: read <file>, list [path], explain <file>, test, run <test_name>");
    println!();

    // Initialize conversation with system message including repository context
    let repo_context = get_repository_context();
    let mcp_info = if std::env::var("ARKAVO_MCP_ENABLED").unwrap_or_default() == "true" {
        "\n\nMCP Integration: Enabled\nYou can run tests and interact with iOS simulators through MCP tools."
    } else {
        "\n\nMCP Integration: Disabled\nTo enable MCP tools, set ARKAVO_MCP_ENABLED=true"
    };

    let system_prompt = format!(
        "You are Arkavo, an AI assistant helping with software development tasks. \
         You have access to the current repository context and can help with code, \
         testing, and development workflows.

\
         Repository context:
{}{}",
        repo_context, mcp_info
    );
    let mut messages = vec![Message::system(&system_prompt)];

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

        // Check for file operation commands
        if let Some(command_response) = handle_command(input) {
            println!("Assistant: {}", command_response);
            println!();
            continue;
        }

        // Check if input contains explain command - enhance with file context
        let enhanced_input = if input.starts_with("explain ") {
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() >= 2 {
                let file_path = parts[1..].join(" ");
                if let Some(content) = read_file(&file_path) {
                    format!("{}\n\nFile content:\n{}", input, content)
                } else {
                    input.to_string()
                }
            } else {
                input.to_string()
            }
        } else {
            input.to_string()
        };

        // Add user message
        messages.push(Message::user(&enhanced_input));

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

fn get_repository_context() -> String {
    let current_dir = env::current_dir().unwrap_or_default();
    let mut context = String::new();

    // Get basic repository info
    context.push_str(&format!("Working directory: {}\n", current_dir.display()));

    // Check if it's a git repository
    if Path::new(".git").exists() {
        context.push_str("Git repository: Yes\n");

        // Get current branch
        if let Ok(output) = std::process::Command::new("git")
            .args(["branch", "--show-current"])
            .output()
        {
            if output.status.success() {
                let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
                context.push_str(&format!("Current branch: {}\n", branch));
            }
        }
    } else {
        context.push_str("Git repository: No\n");
    }

    // List key project files
    context.push_str("\nProject structure:\n");
    if let Ok(entries) = fs::read_dir(&current_dir) {
        let mut files: Vec<String> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        files.sort();

        for file in files.iter().take(20) {
            context.push_str(&format!("  - {}\n", file));
        }

        if files.len() > 20 {
            context.push_str(&format!("  ... and {} more files\n", files.len() - 20));
        }
    }

    // Check for common project files
    let project_files = vec![
        "Cargo.toml",
        "package.json",
        "README.md",
        "requirements.txt",
    ];
    for file in project_files {
        if Path::new(file).exists() {
            context.push_str(&format!("\nDetected project type: {}\n", file));
            break;
        }
    }

    context
}

fn handle_command(input: &str) -> Option<String> {
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    match parts[0] {
        "read" | "cat" => {
            if parts.len() < 2 {
                return Some("Usage: read <file_path>".to_string());
            }
            let file_path = parts[1..].join(" ");
            read_file(&file_path)
        }
        "list" | "ls" => {
            let path = if parts.len() > 1 {
                parts[1..].join(" ")
            } else {
                ".".to_string()
            };
            list_files(&path)
        }
        "explain" => {
            if parts.len() < 2 {
                return Some("Usage: explain <file_path>".to_string());
            }
            // This will be handled by the LLM with file context
            None
        }
        "test" => {
            if std::env::var("ARKAVO_MCP_ENABLED").unwrap_or_default() != "true" {
                return Some("MCP integration is disabled. Set ARKAVO_MCP_ENABLED=true to enable test commands.".to_string());
            }
            // Let the LLM handle test requests through MCP
            None
        }
        "run" => {
            if parts.len() < 2 {
                return Some("Usage: run <test_name>".to_string());
            }
            if std::env::var("ARKAVO_MCP_ENABLED").unwrap_or_default() != "true" {
                return Some("MCP integration is disabled. Set ARKAVO_MCP_ENABLED=true to enable test commands.".to_string());
            }
            // Let the LLM handle test execution through MCP
            None
        }
        _ => None,
    }
}

fn read_file(file_path: &str) -> Option<String> {
    match fs::read_to_string(file_path) {
        Ok(content) => {
            let lines: Vec<&str> = content.lines().collect();
            let preview = if lines.len() > 50 {
                format!(
                    "{}\n\n... (showing first 50 lines of {} total lines)",
                    lines[..50].join("\n"),
                    lines.len()
                )
            } else {
                content
            };
            Some(format!("Content of {}:\n\n{}", file_path, preview))
        }
        Err(e) => Some(format!("Error reading file '{}': {}", file_path, e)),
    }
}

fn list_files(path: &str) -> Option<String> {
    let path = Path::new(path);

    match fs::read_dir(path) {
        Ok(entries) => {
            let mut files = Vec::new();
            let mut dirs = Vec::new();

            for entry in entries.filter_map(|e| e.ok()) {
                let file_name = entry.file_name().to_string_lossy().to_string();
                if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                    dirs.push(format!("{}/ (dir)", file_name));
                } else {
                    files.push(file_name);
                }
            }

            dirs.sort();
            files.sort();

            let mut result = format!("Contents of {}:\n\n", path.display());

            for dir in &dirs {
                result.push_str(&format!("  {}\n", dir));
            }

            for file in &files {
                result.push_str(&format!("  {}\n", file));
            }

            if dirs.is_empty() && files.is_empty() {
                result.push_str("  (empty directory)");
            }

            Some(result)
        }
        Err(e) => Some(format!(
            "Error listing directory '{}': {}",
            path.display(),
            e
        )),
    }
}
