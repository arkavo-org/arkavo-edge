use crate::mcp_integration::McpConnection;
use arkavo_llm::{LlmClient, Message, encode_image_file};
use serde_json::json;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use tokio::runtime::Runtime;
use tokio_stream::StreamExt;
use std::sync::atomic::AtomicBool;

// Global flag to control whether to show debug messages (kept for future use)
#[allow(dead_code)]
static SHOW_DEBUG: AtomicBool = AtomicBool::new(true);

// Macro that does nothing - removes all DEBUG messages
macro_rules! debug_println {
    ($($arg:tt)*) => {
        // Do nothing
    };
}

pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // Check if there's a --prompt argument (also accepts --print for compatibility)
    let prompt = args
        .windows(2)
        .find(|w| w[0] == "--prompt" || w[0] == "--print")
        .map(|w| w[1].clone());

    // Check if there's an --image argument
    let image_path = args
        .windows(2)
        .find(|w| w[0] == "--image")
        .map(|w| w[1].clone());

    // Check if --print or --prompt flag is present (print mode is enabled when prompt is provided)
    let print_mode = args.contains(&"--print".to_string()) 
        || args.contains(&"--prompt".to_string())
        || prompt.is_some();

    // Create runtime for async operations
    let runtime = Runtime::new()?;

    // Initialize LLM client
    let client = runtime.block_on(async {
        LlmClient::from_env().map_err(|e| format!("Failed to initialize LLM client: {}", e))
    })?;

    if !print_mode {
        println!("Starting UI testing chat session...");
        println!("Repository context: {}", get_current_directory());
        println!("LLM Provider: {}", client.provider_name());
        println!("Type '/exit' or '/quit' to end the session.");
        println!(
            "Commands: /read <file>, /list [path], /test, /run <test_name>, /tools"
        );
        println!("Vision commands: @screenshot <path> - Analyze a screenshot");
    }

    // Initialize MCP client - attempt by default unless explicitly disabled
    let mcp_client = if std::env::var("ARKAVO_MCP_DISABLED").unwrap_or_default() != "true" {
        let mcp_url = std::env::var("ARKAVO_MCP_URL").ok();
        let result = match mcp_url {
            Some(url) => McpConnection::new_external(Some(url)),
            None => McpConnection::new_in_process(),
        };
        
        match result {
            Ok(client) => {
                if !print_mode {
                    match &client {
                        McpConnection::InProcess(_) => eprintln!("✓ Using in-process MCP server"),
                        McpConnection::External(_) => eprintln!("✓ Connected to external MCP server"),
                    }
                }
                Some(client)
            }
            Err(_e) => {
                if !print_mode {
                    eprintln!("ℹ MCP server not available - using LLM-only mode");
                }
                None
            }
        }
    } else {
        None
    };

    // Show MCP tools help if connected
    if !print_mode && mcp_client.is_some() {
        println!("MCP tools: @<toolname> [args] - Invoke MCP tool directly");
        println!();
    }

    // Initialize conversation with system message including repository context
    let repo_context = get_repository_context();
    let mcp_info = if mcp_client.is_some() {
        // List available tools
        if let Some(ref client) = mcp_client {
            match client.list_tools() {
                Ok(tools) => {
                    if tools.is_empty() {
                        eprintln!("Warning: No MCP tools returned from server");
                        "\n\nMCP Integration: Enabled\nNo tools available yet. Use /tools command to refresh.".to_string()
                    } else {
                        let mut tool_info = String::from("\n\nMCP Integration: Enabled\n\nAvailable MCP tools:\n");

                        // Group tools by category for better organization
                        let mut device_tools = Vec::new();
                        let mut ui_tools = Vec::new();
                        let mut test_tools = Vec::new();
                        let mut other_tools = Vec::new();

                        for tool in &tools {
                            let tool_desc = format!("- @{}: {}", tool.name, tool.description);

                            if tool.name.contains("device") || tool.name.contains("simulator") {
                                device_tools.push(tool_desc);
                            } else if tool.name.contains("ui_") || tool.name.contains("screen") || tool.name == "analyze_screenshot" {
                                ui_tools.push(tool_desc);
                            } else if tool.name.contains("test") || tool.name == "run_test" || tool.name == "list_tests" {
                                test_tools.push(tool_desc);
                            } else {
                                other_tools.push(tool_desc);
                            }
                        }

                        if !device_tools.is_empty() {
                            tool_info.push_str("\nDevice Management:\n");
                            tool_info.push_str(&device_tools.join("\n"));
                        }

                        if !ui_tools.is_empty() {
                            tool_info.push_str("\n\nUI Interaction:\n");
                            tool_info.push_str(&ui_tools.join("\n"));
                        }

                        if !test_tools.is_empty() {
                            tool_info.push_str("\n\nTesting:\n");
                            tool_info.push_str(&test_tools.join("\n"));
                        }

                        if !other_tools.is_empty() {
                            tool_info.push_str("\n\nOther Tools:\n");
                            tool_info.push_str(&other_tools.join("\n"));
                        }

                        tool_info
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Failed to list MCP tools: {}", e);
                    "\n\nMCP Integration: Enabled (tool listing failed)\nYou can run tests and interact with iOS simulators through MCP tools. Use /tools command to see available tools.".to_string()
                }
            }
        } else {
            "\n\nMCP Integration: Enabled\nYou can run tests and interact with iOS simulators through MCP tools.".to_string()
        }
    } else {
        "\n\nMCP Integration: Disabled\nTo enable MCP tools, run 'arkavo serve' in another terminal"
            .to_string()
    };

    let system_prompt = format!(
        "You are an expert UI testing assistant working with the Arkavo Edge project. \
         You have access to MCP tools for clicking elements, entering text, and other UI interactions. \
         When the user asks you to test something, you should use the appropriate MCP tools to interact with the UI. \
         Always analyze images thoroughly to understand the current state of the UI before suggesting next steps.

\
         To invoke an MCP tool, use the format: @toolname {{arguments}} or @toolname plain text arguments\
         For example: @device_management {{\"action\": \"list\"}} or @ui_interaction {{\"action\": \"tap\", \"element\": \"button\"}}

\
         TYPICAL UI TESTING WORKFLOW:\
         1. Use @device_management {{\"action\": \"list\"}} to find available devices\
         2. Use @screen_capture {{\"device_id\": \"<device_id>\"}} to take a screenshot\
         3. The screenshot path will be returned, which you can then analyze using vision capabilities\
         4. Use @ui_interaction for tapping, swiping, or entering text based on what you see

\
         When a user asks to analyze an image, you should:\
         - Use @analyze_screenshot with the path to analyze a screenshot: @analyze_screenshot path/to/screenshot.png\
         - Or use your vision capabilities to analyze the provided image\
         - Describe what you see in detail\
         - Suggest appropriate UI interactions based on the content

\
         Repository context:
{}{}",
        repo_context, mcp_info
    );
    let mut messages = vec![Message::system(&system_prompt)];

    // If prompt provided via command line, process it and exit
    if let Some(prompt_text) = prompt {
        // Check if image is also provided
        if let Some(img_path) = image_path {
            match encode_image_file(&img_path) {
                Ok(encoded_image) => {
                    messages.push(Message::user_with_images(&prompt_text, vec![encoded_image]));
                }
                Err(e) => {
                    eprintln!("Error loading image: {}", e);
                    messages.push(Message::user(&prompt_text));
                }
            }
        } else {
            messages.push(Message::user(&prompt_text));
        }

        if print_mode {
            runtime.block_on(process_message_print(&client, &messages, &mcp_client))?;
        } else {
            runtime.block_on(process_message(&client, &messages, &mcp_client))?;
        }
        return Ok(());
    }

    // Interactive chat loop
    loop {
        print!("> ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        let input = input.trim();
        debug_println!("DEBUG: User input: '{}'", input);
        if input.is_empty() {
            continue;
        }

        if input == "/exit" || input == "/quit" || input == "exit" || input == "quit" {
            println!("Exiting chat session.");
            break;
        }

        if input == "clear" {
            // Keep only system message
            messages.truncate(1);
            println!("Conversation cleared.");
            continue;
        }

        // Check for @tool syntax at the beginning of input
        if input.starts_with('@') && mcp_client.is_some() {
            let parts: Vec<&str> = input.splitn(2, ' ').collect();
            if parts.len() >= 1 {
                let tool_name = &parts[0][1..]; // Remove @ prefix
                let args_str = if parts.len() > 1 { parts[1] } else { "" };

                // Try to parse arguments as JSON, or create a simple prompt object
                let args = if args_str.trim().starts_with('{') {
                    serde_json::from_str(args_str).unwrap_or_else(|_| json!({"prompt": args_str}))
                } else {
                    json!({"prompt": args_str})
                };

                if let Some(ref mcp) = mcp_client {
                    match mcp.call_tool(tool_name, args, client.provider_name()) {
                        Ok(result) => {
                            println!("Tool Result ({}):", tool_name);
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&result)
                                    .unwrap_or_else(|_| result.to_string())
                            );
                            println!();

                            // Add to conversation context
                            messages.push(Message::user(input));
                            messages.push(Message::assistant(&format!(
                                "Tool {} executed. Result: {}",
                                tool_name, result
                            )));
                        }
                        Err(e) => {
                            eprintln!("Tool execution failed: {}", e);
                        }
                    }
                    continue;
                }
            }
        }

        // Check for slash commands
        if input.starts_with('/') {
            let command_input = &input[1..]; // Remove the slash
            if let Some(command_response) = handle_command(command_input, &mcp_client, client.provider_name()) {
                println!("{}", command_response);
                println!();
                continue;
            }
        }

        // Check for @screenshot command without arguments
        if input == "@screenshot" {
            eprintln!("Usage: @screenshot <path>");
            eprintln!("Note: The 'screenshot' tool is not available for direct LLM use. Please provide a path to an existing image file.");
            continue;
        }
        // Check for @screenshot command anywhere in the input
        else if let Some(screenshot_pos) = input.find("@screenshot ") {
            // Extract the path after @screenshot
            let after_command = &input[screenshot_pos + "@screenshot ".len()..];
            let img_path = after_command.trim();

            if !img_path.is_empty() {
                match encode_image_file(img_path) {
                    Ok(encoded_image) => {
                        // Use the text before @screenshot as the prompt, or a default
                        let prompt = if screenshot_pos > 0 {
                            input[..screenshot_pos].trim()
                        } else {
                            "Analyze this screenshot and describe what you see. Focus on UI elements, their states, and any notable features."
                        };
                        messages.push(Message::user_with_images(prompt, vec![encoded_image]));
                    }
                    Err(e) => {
                        eprintln!("Error loading screenshot: {}", e);
                        continue;
                    }
                }
            } else {
                eprintln!("Usage: @screenshot <path>");
                continue;
            }
        }
        // Check for "analyze_screenshot on path" syntax and convert it to "@analyze_screenshot path"
        else if let Some(analyze_pos) = input.find("analyze_screenshot on ") {
            // Extract the path after "analyze_screenshot on"
            let after_command = &input[analyze_pos + "analyze_screenshot on ".len()..];
            let img_path = after_command.trim();

            if !img_path.is_empty() {
                // Convert to "@analyze_screenshot path" syntax
                let converted_input = format!("@analyze_screenshot {}", img_path);
                messages.push(Message::user(&converted_input));
            } else {
                eprintln!("Usage: analyze_screenshot on <path>");
                continue;
            }
        } else {
            // Add regular user message
            messages.push(Message::user(input));
        }

        // Process with LLM
        match runtime.block_on(process_message(&client, &messages, &mcp_client)) {
            Ok(response) => {
                messages.push(Message::assistant(&response));

                // If the response contains tool execution results, we might need to continue the conversation
                if response.contains("[Tool execution completed. Results shown above.]") {
                    // The tool results have been displayed
                }
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
    mcp_client: &Option<McpConnection>,
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

    // Check if the response contains @tool calls and execute them
    if let Some(mcp) = mcp_client {
        debug_println!("DEBUG: Checking LLM response for tool calls. Response length: {}", full_response.len());
        debug_println!("DEBUG: First 200 chars of response: {}", &full_response.chars().take(200).collect::<String>());

        let (response_text, tool_results) = handle_tool_calls_in_response(&full_response, mcp, client.provider_name())?;

        debug_println!("DEBUG: Tool results count: {}", tool_results.len());

        // If we executed tools, display them nicely
        if !tool_results.is_empty() {
            println!(); // Extra line before tool results
            println!("=== MCP Tool Results ===");

            for (tool_name, result) in &tool_results {
                println!("\n[Tool: {}]", tool_name);
                println!("Response:");

                // Pretty print the result if it's JSON
                if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&result) {
                    if let Ok(pretty) = serde_json::to_string_pretty(&json_val) {
                        println!("{}", pretty);
                    } else {
                        println!("{}", result);
                    }
                } else {
                    println!("{}", result);
                }
            }

            println!("\n=== End Tool Results ===\n");

            // Now continue the conversation with the tool results
            // Add the tool results to the response for context
            let mut response_with_results = response_text.clone();
            response_with_results.push_str("\n\n[Tool execution completed. Results shown above.]");

            return Ok(response_with_results);
        }
    }

    println!(); // Extra line for readability

    Ok(full_response)
}

async fn process_message_print(
    client: &LlmClient,
    messages: &[Message],
    mcp_client: &Option<McpConnection>,
) -> Result<String, Box<dyn std::error::Error>> {
    // Use streaming but only print content
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

    println!(); // New line at end

    // Check if the response contains @tool calls and execute them
    if let Some(mcp) = mcp_client {
        let (response_text, tool_results) = handle_tool_calls_in_response(&full_response, mcp, client.provider_name())?;

        // If we executed tools, print them
        if !tool_results.is_empty() {
            for (tool_name, result) in tool_results {
                println!("\n[Tool Result - {}]:", tool_name);

                // Pretty print the result if it's JSON
                if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&result) {
                    if let Ok(pretty) = serde_json::to_string_pretty(&json_val) {
                        println!("{}", pretty);
                    } else {
                        println!("{}", result);
                    }
                } else {
                    println!("{}", result);
                }
            }
            io::stdout().flush()?;
            return Ok(response_text);
        }
    }

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

fn handle_command(
    input: &str,
    mcp_client: &Option<McpConnection>,
    llm_provider: &str,
) -> Option<String> {
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

            // Use MCP if available
            if let Some(client) = mcp_client {
                match client.call_tool("read_file", json!({ "path": file_path }), llm_provider) {
                    Ok(result) => {
                        if let Some(text) = result.get("result").and_then(|r| r.as_str()) {
                            Some(format!("Content of {} (via MCP):\n\n{}", file_path, text))
                        } else {
                            Some(format!("MCP read result: {}", result))
                        }
                    }
                    Err(e) => {
                        eprintln!("MCP read failed, falling back to local: {}", e);
                        read_file(&file_path)
                    }
                }
            } else {
                read_file(&file_path)
            }
        }
        "list" | "ls" => {
            let path = if parts.len() > 1 {
                parts[1..].join(" ")
            } else {
                ".".to_string()
            };

            // Use MCP if available
            if let Some(client) = mcp_client {
                match client.call_tool("list_directory", json!({ "path": path }), llm_provider) {
                    Ok(result) => {
                        if let Some(text) = result.get("result").and_then(|r| r.as_str()) {
                            Some(format!("Contents of {} (via MCP):\n\n{}", path, text))
                        } else {
                            Some(format!("MCP list result: {}", result))
                        }
                    }
                    Err(e) => {
                        eprintln!("MCP list failed, falling back to local: {}", e);
                        list_files(&path)
                    }
                }
            } else {
                list_files(&path)
            }
        }
        "test" => {
            if mcp_client.is_none() {
                return Some(
                    "MCP server not available. Run 'arkavo serve' to enable test commands."
                        .to_string(),
                );
            }

            if let Some(client) = mcp_client {
                match client.call_tool("list_tests", json!({}), llm_provider) {
                    Ok(result) => {
                        if let Some(text) = result.get("result").and_then(|r| r.as_str()) {
                            Some(format!("Available tests (via MCP):\n\n{}", text))
                        } else {
                            Some(format!("MCP test list result: {}", result))
                        }
                    }
                    Err(e) => Some(format!("Failed to list tests: {}", e)),
                }
            } else {
                None
            }
        }
        "run" => {
            if parts.len() < 2 {
                return Some("Usage: run <test_name>".to_string());
            }
            if mcp_client.is_none() {
                return Some(
                    "MCP server not available. Run 'arkavo serve' to enable test commands."
                        .to_string(),
                );
            }

            let test_name = parts[1..].join(" ");
            if let Some(client) = mcp_client {
                match client.call_tool("run_test", json!({ "test_name": test_name }), llm_provider)
                {
                    Ok(result) => {
                        if let Some(text) = result.get("result").and_then(|r| r.as_str()) {
                            Some(format!("Test execution result (via MCP):\n\n{}", text))
                        } else {
                            Some(format!("MCP test result: {}", result))
                        }
                    }
                    Err(e) => Some(format!("Failed to run test: {}", e)),
                }
            } else {
                None
            }
        }
        "tools" => {
            if let Some(client) = mcp_client {
                match client.list_tools() {
                    Ok(tools) => {
                        if tools.is_empty() {
                            Some("No MCP tools available. The server may not have returned tools in the expected format.".to_string())
                        } else {
                            let mut output = "Available MCP tools:\n\n".to_string();
                            for tool in tools {
                                output.push_str(&format!("  {} - {}\n", tool.name, tool.description));
                            }
                            Some(output)
                        }
                    }
                    Err(e) => Some(format!("Failed to list MCP tools: {}", e)),
                }
            } else {
                Some(
                    "MCP server not available. Run 'arkavo serve' to enable MCP tools.".to_string(),
                )
            }
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

fn handle_tool_calls_in_response(
    response: &str,
    mcp_client: &McpConnection,
    llm_provider: &str,
) -> Result<(String, Vec<(String, String)>), Box<dyn std::error::Error>> {
    // Find all @tool calls in the response
    let mut tool_results = Vec::new();

    // Return the original response text to avoid interrupting the flow
    let original_response = response.to_string();

    // Use a more robust approach to find @tool calls
    // First, remove markdown code blocks to find tools within them
    let cleaned_response = response
        .replace("```", "")
        .replace("`", "");

    debug_println!("DEBUG: Cleaned response first 200 chars: {}", &cleaned_response.chars().take(200).collect::<String>());

    let remaining = &cleaned_response[..];
    let mut found_tools = 0;

    debug_println!("DEBUG: Starting tool detection in cleaned response");

    // Process only the first tool call to avoid interrupting the flow
    // This allows for multi-tasking by processing one tool at a time
    if let Some(at_pos) = remaining.find('@') {
        // Check if this is a tool call (followed by word characters)
        let after_at = &remaining[at_pos + 1..];
        debug_println!("DEBUG: Found @ symbol at position {}, text after @: '{}'", at_pos, &after_at.chars().take(20).collect::<String>());

        if let Some(space_or_brace) = after_at.find(|c: char| c.is_whitespace() || c == '{') {
            let tool_name = &after_at[..space_or_brace];
            debug_println!("DEBUG: Potential tool name: '{}'", tool_name);

            // Only process if tool_name is alphanumeric and not exactly "screenshot" (which is not allowed)
            if tool_name.chars().all(|c| c.is_alphanumeric() || c == '_') && tool_name != "screenshot" {
                found_tools += 1;
                debug_println!("DEBUG: Valid tool found: '{}' (tool #{} in response)", tool_name, found_tools);

                let args_start = at_pos + 1 + space_or_brace;
                let args_str = &remaining[args_start..].trim_start();
                debug_println!("DEBUG: Arguments start: '{}'", &args_str.chars().take(30).collect::<String>());

                let (args, _consumed_len) = if args_str.starts_with('{') {
                    // Find matching closing brace
                    let mut brace_count = 0;
                    let mut end_pos = 0;
                    for (i, ch) in args_str.chars().enumerate() {
                        match ch {
                            '{' => brace_count += 1,
                            '}' => {
                                brace_count -= 1;
                                if brace_count == 0 {
                                    end_pos = i + 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }

                    if end_pos > 0 {
                        let json_str = &args_str[..end_pos];
                        debug_println!("DEBUG: Attempting to parse JSON arguments: '{}'", json_str);
                        match serde_json::from_str(json_str) {
                            Ok(json) => {
                                debug_println!("DEBUG: Successfully parsed JSON arguments");
                                (json, end_pos)
                            },
                            Err(_e) => {
                                debug_println!("DEBUG: Failed to parse JSON arguments: {}", _e);
                                debug_println!("DEBUG: Falling back to using raw text as prompt");
                                (json!({"prompt": json_str}), end_pos)
                            },
                        }
                    } else {
                        debug_println!("DEBUG: No closing brace found, using entire string as prompt");
                        (json!({"prompt": args_str}), 0)
                    }
                } else {
                    // Take until newline or end of string
                    let end_pos = args_str.find('\n').unwrap_or(args_str.len());
                    let arg_text = &args_str[..end_pos].trim();
                    debug_println!("DEBUG: Using plain text as arguments: '{}'", arg_text);
                    (json!({"prompt": arg_text}), end_pos)
                };

                debug_println!("DEBUG: About to execute tool {} with args: {:?}", tool_name, args);

                // Execute the tool
                match mcp_client.call_tool(tool_name, args, llm_provider) {
                    Ok(tool_result) => {
                        debug_println!("DEBUG: Tool {} returned: {:?}", tool_name, tool_result);

                        // Extract the actual result text from the MCP response
                        debug_println!("DEBUG: Extracting result text from tool response");
                        let result_text = if let Some(result_obj) = tool_result.get("result") {
                            if let Some(text) = result_obj.as_str() {
                                debug_println!("DEBUG: Found string result in 'result' field");
                                text.to_string()
                            } else {
                                debug_println!("DEBUG: 'result' field is not a string, converting to JSON");
                                serde_json::to_string_pretty(&result_obj).unwrap_or_else(|_e| {
                                    debug_println!("DEBUG: Failed to convert result to JSON: {}", _e);
                                    result_obj.to_string()
                                })
                            }
                        } else {
                            debug_println!("DEBUG: No 'result' field found, using entire response");
                            serde_json::to_string_pretty(&tool_result).unwrap_or_else(|_e| {
                                debug_println!("DEBUG: Failed to convert entire response to JSON: {}", _e);
                                tool_result.to_string()
                            })
                        };

                        debug_println!("DEBUG: Extracted result text: {}", result_text);
                        tool_results.push((tool_name.to_string(), result_text));
                    }
                    Err(e) => {
                        debug_println!("DEBUG: Tool {} failed with error: {}", tool_name, e);
                        tool_results.push((tool_name.to_string(), format!("Error: {}", e)));
                    }
                }
            } else {
                debug_println!("DEBUG: Tool name '{}' rejected - not alphanumeric", tool_name);
            }
        } else {
            debug_println!("DEBUG: No space or brace after @ symbol - not a valid tool call");
        }
    }

    debug_println!("DEBUG: Found {} tools in response, executed {} tools", found_tools, tool_results.len());

    if found_tools == 0 {
        debug_println!("DEBUG: No tools were detected in the response");
    } else if found_tools != tool_results.len() {
        debug_println!("DEBUG: Warning - {} tools were detected but only {} were successfully executed", 
                 found_tools, tool_results.len());
    }

    // Print a summary of the entire pipeline
    debug_println!("\nDEBUG: TOOL CALLING PIPELINE SUMMARY:");
    debug_println!("DEBUG: 1. Cleaned response length: {} chars", cleaned_response.len());
    debug_println!("DEBUG: 2. Tools found in response: {}", found_tools);
    debug_println!("DEBUG: 3. Tools successfully executed: {}", tool_results.len());
    debug_println!("DEBUG: 4. Final tool results count: {}", tool_results.len());

    // Return the original response text to avoid interrupting the flow
    Ok((original_response, tool_results))
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
