use crate::conversation_manager::ConversationManager;
#[cfg(all(unix, feature = "test-harness"))]
use crate::mcp_integration::McpConnection;
use arkavo_llm::{LlmClient, Message, encode_image_file};
use arkavo_memory::storage::MemoryStorage;
use arkavo_repo::repository_context::RepositoryContextManager;
use chrono;
use indicatif::{ProgressBar, ProgressStyle};
use serde_json::json;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::runtime::Runtime;
use tokio_stream::StreamExt;
use uuid;

// Global flag to control whether to show debug messages (kept for future use)
#[allow(dead_code)]
static SHOW_DEBUG: AtomicBool = AtomicBool::new(true);

// Create placeholder types when MCP is not available
#[cfg(not(all(unix, feature = "test-harness")))]
struct McpConnection;

#[cfg(not(all(unix, feature = "test-harness")))]
#[derive(Debug)]
struct Tool {
    name: String,
    description: String,
}

#[cfg(not(all(unix, feature = "test-harness")))]
impl McpConnection {
    fn list_tools(&self) -> Result<Vec<Tool>, Box<dyn std::error::Error>> {
        Ok(Vec::new())
    }

    fn call_tool(
        &self,
        _tool_name: &str,
        _args: serde_json::Value,
        _provider: &str,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        Err("MCP tools not available in this build".into())
    }
}

// Runtime MCP initialization - checks if test-harness feature is available
#[cfg(all(target_os = "macos", feature = "test-harness"))]
fn initialize_mcp_connection(print_mode: bool) -> Option<McpConnection> {
    // Try in-process MCP first, which includes all local tools
    let result = McpConnection::new_in_process();

    match result {
        Ok(client) => {
            if !print_mode {
                match &client {
                    McpConnection::InProcess(_) => {
                        eprintln!("✓ Using in-process MCP server");
                    }
                    McpConnection::External(_) => {
                        eprintln!("✓ Connected to external MCP server");
                    }
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
}

#[cfg(not(all(target_os = "macos", feature = "test-harness")))]
fn initialize_mcp_connection(print_mode: bool) -> Option<McpConnection> {
    // On non-macOS platforms or without test-harness, try external MCP connection
    // Check for MCP_URL environment variable or use default
    let mcp_url = std::env::var("MCP_URL").ok();

    match McpConnection::new_external(mcp_url) {
        Ok(client) => {
            if !print_mode {
                eprintln!("✓ Connected to external MCP server");
            }
            Some(client)
        }
        Err(_) => {
            if !print_mode {
                eprintln!("ℹ MCP server not available - using LLM-only mode");
            }
            None
        }
    }
}

#[allow(clippy::disallowed_methods)]
pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // Terminal UI is now the default, use --no-tui to disable it
    let use_tui = !args.contains(&"--no-tui".to_string());

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

    // Launch Terminal UI early if requested and not in print mode
    if use_tui && !print_mode {
        // For TUI mode, we'll initialize everything inside the TUI
        return launch_terminal_ui(runtime);
    }

    // Initialize memory storage
    let memory_storage = Arc::new(runtime.block_on(MemoryStorage::new())?);

    // Initialize conversation manager
    let mut conversation_manager = ConversationManager::new(memory_storage.clone())?;

    // Initialize repository context manager
    let _repo_context_manager = RepositoryContextManager::new(memory_storage)?;

    // Initialize LLM client with fallback to prompt for remote server
    let client = runtime.block_on(initialize_llm_client(print_mode))?;

    if !print_mode {
        println!("Starting UI testing chat session...");
        println!("Repository context: {}", get_current_directory());
        println!("LLM Provider: {}", client.provider_name());

        // Try to restore last session
        if let Ok(Some(session_id)) = runtime.block_on(conversation_manager.restore_last_session())
        {
            println!(
                "Restored previous conversation (session: {})",
                &session_id.to_string()[..8]
            );
        } else {
            // Start new session
            let _ = runtime.block_on(conversation_manager.start_session(client.provider_name()))?;
        }

        println!("Type '/exit' or '/quit' to end the session.");
        println!(
            "Commands: /read <file>, /list [path], /test, /run <test_name>, /tools, /switch <session>"
        );
        println!("Vision commands: @screenshot <path> - Analyze a screenshot");
    }

    // Initialize MCP client - skip in print mode, otherwise attempt connection
    #[allow(unused_variables)]
    let mcp_client: Option<McpConnection> = if print_mode {
        None
    } else {
        // Try to initialize MCP if available
        initialize_mcp_connection(print_mode)
    };

    // Show MCP tools help if connected
    if !print_mode && mcp_client.is_some() {
        println!("MCP tools: @<toolname> [args] - Invoke MCP tool directly");
        println!();
    }

    /*
    // Build enhanced repository context
    let progress = ProgressBar::new_spinner();
    progress.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    progress.set_message("Building repository context...");

    let repo_context = runtime.block_on(repo_context_manager.build_context())?;
    let repo_context_str = format!(
        "Working directory: {}\n\
         Git repository: {}\n\
         Current branch: {}\n\
         Project type: {}\n\
         Total files: {}\n\
         Token count: {}",
        repo_context.working_directory,
        repo_context.is_git_repo,
        repo_context.current_branch.as_deref().unwrap_or("N/A"),
        repo_context.project_type.as_deref().unwrap_or("Unknown"),
        repo_context.project_files.len(),
        repo_context.token_count
    );

    progress.finish_and_clear();
    */

    // Initialize with a minimal context and let the agent ask for more.
    let repo_context_str = format!("Working directory: {}", get_current_directory());
    let repo_context = json!({
        "working_directory": get_current_directory(),
    });

    // Initialize conversation with system message including repository context
    let mcp_info = if mcp_client.is_some() {
        // List available tools
        if let Some(ref client) = mcp_client {
            match client.list_tools() {
                Ok(tools) => {
                    if tools.is_empty() {
                        eprintln!("Warning: No MCP tools returned from server");
                        "\n\nMCP Integration: Enabled\nNo tools available yet. Use /tools command to refresh.".to_string()
                    } else {
                        let mut tool_info =
                            String::from("\n\nMCP Integration: Enabled\n\nAvailable MCP tools:\n");

                        // Group tools by category for better organization
                        let mut device_tools = Vec::new();
                        let mut ui_tools = Vec::new();
                        let mut test_tools = Vec::new();
                        let mut other_tools = Vec::new();

                        for tool in &tools {
                            let tool_desc = format!("- @{}: {}", tool.name, tool.description);

                            if tool.name.contains("device") || tool.name.contains("simulator") {
                                device_tools.push(tool_desc);
                            } else if tool.name.contains("ui_")
                                || tool.name.contains("screen")
                                || tool.name == "analyze_screenshot"
                            {
                                ui_tools.push(tool_desc);
                            } else if tool.name.contains("test")
                                || tool.name == "run_test"
                                || tool.name == "list_tests"
                            {
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
                    eprintln!("Warning: Failed to list MCP tools: {e}");
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

    // Try to read AGENTS.md for system prompt
    let agents_md_content = match std::fs::read_to_string("AGENTS.md") {
        Ok(content) => Some(content),
        Err(_) => {
            // Try CLAUDE.md as fallback
            std::fs::read_to_string("CLAUDE.md").ok()
        }
    };

    let system_prompt = if let Some(agents_content) = agents_md_content {
        format!(
            "{}\n\nRepository context:\n{}\n\nRepository details:\n{}\n\n{}",
            agents_content,
            repo_context_str,
            serde_json::to_string_pretty(&repo_context).unwrap_or_default(),
            mcp_info
        )
    } else {
        format!(
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
         GIT REPOSITORY ANALYSIS:\
         When asked to perform a \\\"full analysis\\\", \\\"repository analysis\\\", or comprehensive Git analysis:\
         1. MUST call @build_repository_context {{}} to get the full repository context.\
         2. MUST call @git_status {{}} to get working tree status\
         3. MUST call @git_log {{\"limit\": 20}} to get recent commits\
         4. MUST call @git_diff {{}} to get unstaged changes\
         5. MUST call @git_diff {{\"staged\": true}} to get staged changes\
         6. MUST call @git_branch {{\"action\": \"list\"}} to get branch information\
         7. MUST call @git_remote {{\"action\": \"fetch\"}} to check remote status\
         \
         After collecting all responses:\
         - Generate a structured report with sections for each data type\
         - Use ONLY actual data from tool responses - DO NOT fabricate any information\
         - If a tool fails, note the failure in the report\
         - Store the complete analysis in memory using @store_memory

\
         Initial repository context (minimal):
{}

Full repository details (available via @build_repository_context):
{}

{}",
        repo_context_str,
        serde_json::to_string_pretty(&repo_context).unwrap_or_default(),
        mcp_info
    )
    };

    // Get conversation context with system message
    let system_message = if print_mode {
        // Use minimal system prompt for print mode to avoid token limit issues
        Message::system("You are a helpful AI assistant.")
    } else {
        Message::system(&system_prompt)
    };

    let mut messages = if print_mode {
        // In print mode, just create a simple message list
        vec![system_message.clone()]
    } else {
        // In interactive mode, get full context from conversation manager
        runtime.block_on(conversation_manager.get_context_messages(Some(system_message.clone())))?
    };

    // If prompt provided via command line, process it and exit
    if let Some(prompt_text) = prompt {
        // Check if image is also provided
        if let Some(img_path) = image_path {
            match encode_image_file(&img_path) {
                Ok(encoded_image) => {
                    messages.push(Message::user_with_images(&prompt_text, vec![encoded_image]));
                }
                Err(e) => {
                    eprintln!("Error loading image: {e}");
                    messages.push(Message::user(&prompt_text));
                }
            }
        } else {
            messages.push(Message::user(&prompt_text));
        }

        if print_mode {
            runtime.block_on(process_message_print(
                &client,
                &messages,
                mcp_client.as_ref(),
            ))?;
        } else {
            runtime.block_on(process_message(
                &client,
                &messages,
                mcp_client.as_ref(),
                &conversation_manager,
            ))?;
        }
        return Ok(());
    }

    // Launch Terminal UI if requested
    if use_tui && !print_mode {
        // Create channels for communication between TUI and LLM
        let (ui_tx, mut ui_rx) = tokio::sync::mpsc::channel::<String>(100);
        let (llm_tx, llm_rx) = tokio::sync::mpsc::channel::<String>(100);

        // Clone necessary components for the TUI task
        let client = Arc::new(client);
        let client_clone: Arc<LlmClient> = Arc::clone(&client);
        let mut messages_clone = messages.clone();

        // Spawn LLM processing task
        let llm_handle = runtime.spawn(async move {
            eprintln!("[LLM Task] Started, waiting for messages...");
            while let Some(user_input) = ui_rx.recv().await {
                eprintln!("[LLM Task] Received user input: {user_input}");
                // Process the user input with LLM
                let user_message = Message::user(user_input.clone());
                messages_clone.push(user_message);

                // Get streaming response from LLM
                match client_clone.stream(messages_clone.clone()).await {
                    Ok(mut stream) => {
                        let mut full_response = String::new();

                        // Send start streaming signal
                        let _ = llm_tx.send("<<STREAM_START>>".to_string()).await;

                        while let Some(chunk_result) = stream.next().await {
                            match chunk_result {
                                Ok(chunk) => {
                                    if !chunk.content.is_empty() {
                                        full_response.push_str(&chunk.content);
                                        // Send each chunk as it arrives
                                        let _ = llm_tx
                                            .send(format!("<<STREAM_CHUNK>>{}", chunk.content))
                                            .await;
                                    }
                                }
                                Err(e) => {
                                    let _ =
                                        llm_tx.send(format!("<<STREAM_ERROR>>Error: {e}")).await;
                                    break;
                                }
                            }
                        }

                        // Send end streaming signal
                        let _ = llm_tx.send("<<STREAM_END>>".to_string()).await;

                        // Save the complete message
                        let assistant_message = Message::assistant(full_response.clone());
                        messages_clone.push(assistant_message);
                        eprintln!(
                            "[LLM Task] Response complete, {} chars. Messages in context: {}",
                            full_response.len(),
                            messages_clone.len()
                        );
                    }
                    Err(e) => {
                        eprintln!("[LLM Task] Error: {e}");
                        let _ = llm_tx.send(format!("Error: {e}")).await;
                    }
                }
                eprintln!("[LLM Task] Waiting for next message...");
            }
            eprintln!("[LLM Task] Channel closed, exiting...");
        });

        // Run the Terminal UI with communication channels
        let tui_result = runtime
            .block_on(async { arkavo_terminal::run_with_string_channels(ui_tx, llm_rx).await });

        // Clean up
        llm_handle.abort();

        return tui_result.map_err(std::convert::Into::into);
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

        if input == "/exit" || input == "/quit" || input == "exit" || input == "quit" {
            println!("Exiting chat session.");
            break;
        }

        if input == "clear" {
            // Start new session
            let _ = runtime.block_on(conversation_manager.start_session(client.provider_name()))?;
            messages = runtime.block_on(
                conversation_manager.get_context_messages(Some(system_message.clone())),
            )?;
            println!("Conversation cleared. New session started.");
            continue;
        }

        // Handle /switch command
        if input.starts_with("/switch") {
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() < 2 {
                // List available sessions
                match runtime.block_on(conversation_manager.list_sessions()) {
                    Ok(sessions) => {
                        println!("Available sessions:");
                        for session in sessions.iter().take(10) {
                            println!(
                                "  {} - {} ({})",
                                &session.id.to_string()[..8],
                                session.created_at.format("%Y-%m-%d %H:%M"),
                                session.model
                            );
                        }
                        println!("\nUsage: /switch <session-id>");
                    }
                    Err(e) => eprintln!("Error listing sessions: {e}"),
                }
            } else {
                // Switch to specified session
                let session_id_str = parts[1];
                if let Ok(session_id) = uuid::Uuid::parse_str(session_id_str) {
                    match runtime.block_on(conversation_manager.switch_session(session_id)) {
                        Ok(()) => {
                            messages = runtime.block_on(
                                conversation_manager
                                    .get_context_messages(Some(system_message.clone())),
                            )?;
                            println!("Switched to session: {}", &session_id.to_string()[..8]);
                        }
                        Err(e) => eprintln!("Error switching session: {e}"),
                    }
                } else {
                    eprintln!("Invalid session ID format");
                }
            }
            continue;
        }

        // Check for @tool syntax at the beginning of input
        if input.starts_with('@') && mcp_client.is_some() {
            let parts: Vec<&str> = input.splitn(2, ' ').collect();
            if !parts.is_empty() {
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
                            println!("Tool Result ({tool_name}):");
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&result)
                                    .unwrap_or_else(|_| result.to_string())
                            );
                            println!();

                            // Add to conversation context
                            let user_msg = Message::user(input);
                            let tool_msg = Message::assistant(format!(
                                "Tool {tool_name} executed. Result: {result}"
                            ));
                            runtime.block_on(conversation_manager.add_message(&user_msg))?;
                            runtime.block_on(conversation_manager.add_message(&tool_msg))?;
                            messages.push(user_msg);
                            messages.push(tool_msg);
                        }
                        Err(e) => {
                            eprintln!("Tool execution failed: {e}");
                        }
                    }
                    continue;
                }
            }
        }

        // Check for slash commands
        if let Some(command_input) = input.strip_prefix('/')
            && let Some(command_response) =
                handle_command(command_input, mcp_client.as_ref(), client.provider_name())
        {
            println!("{command_response}");
            println!();
            continue;
        }

        // Check for @screenshot command without arguments
        if input == "@screenshot" {
            eprintln!("Usage: @screenshot <path>");
            eprintln!(
                "Note: The 'screenshot' tool is not available for direct LLM use. Please provide a path to an existing image file."
            );
            continue;
        }
        // Check for @screenshot command anywhere in the input
        else if let Some(screenshot_pos) = input.find("@screenshot ") {
            // Extract the path after @screenshot
            let after_command = &input[screenshot_pos + "@screenshot ".len()..];
            let img_path = after_command.trim();

            if img_path.is_empty() {
                eprintln!("Usage: @screenshot <path>");
                continue;
            }
            match encode_image_file(img_path) {
                Ok(encoded_image) => {
                    // Use the text before @screenshot as the prompt, or a default
                    let prompt = if screenshot_pos > 0 {
                        input[..screenshot_pos].trim()
                    } else {
                        "Analyze this screenshot and describe what you see. Focus on UI elements, their states, and any notable features."
                    };
                    let msg = Message::user_with_images(prompt, vec![encoded_image]);
                    runtime.block_on(conversation_manager.add_message(&msg))?;
                    messages.push(msg);
                }
                Err(e) => {
                    eprintln!("Error loading screenshot: {e}");
                    continue;
                }
            }
        }
        // Check for "analyze_screenshot on path" syntax and convert it to "@analyze_screenshot path"
        else if let Some(analyze_pos) = input.find("analyze_screenshot on ") {
            // Extract the path after "analyze_screenshot on"
            let after_command = &input[analyze_pos + "analyze_screenshot on ".len()..];
            let img_path = after_command.trim();

            if img_path.is_empty() {
                eprintln!("Usage: analyze_screenshot on <path>");
                continue;
            }
            // Convert to "@analyze_screenshot path" syntax
            let converted_input = format!("@analyze_screenshot {img_path}");
            let msg = Message::user(&converted_input);
            runtime.block_on(conversation_manager.add_message(&msg))?;
            messages.push(msg);
        } else {
            // Add regular user message
            let msg = Message::user(input);
            runtime.block_on(conversation_manager.add_message(&msg))?;
            messages.push(msg);
        }

        // Process with LLM
        match runtime.block_on(process_message(
            &client,
            &messages,
            mcp_client.as_ref(),
            &conversation_manager,
        )) {
            Ok(response) => {
                let assistant_msg = Message::assistant(&response);
                runtime.block_on(conversation_manager.add_message(&assistant_msg))?;
                messages.push(assistant_msg);

                // If the response contains tool execution results, we might need to continue the conversation
                if response.contains("[Tool execution completed. Results shown above.]") {
                    // The tool results have been displayed
                }
            }
            Err(e) => {
                eprintln!("Error: {e}");
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
    mcp_client: Option<&McpConnection>,
    _conversation_manager: &ConversationManager,
) -> Result<String, Box<dyn std::error::Error>> {
    print!("Assistant: ");
    io::stdout().flush()?;

    // Use streaming for better UX
    let progress = ProgressBar::new_spinner();
    progress.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    progress.set_message("Thinking...");

    let mut stream = client.stream(messages.to_vec()).await?;
    let mut full_response = String::new();
    let mut first_token = true;

    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(response) => {
                if first_token {
                    progress.finish_and_clear();
                    first_token = false;
                }
                print!("{}", response.content);
                io::stdout().flush()?;
                full_response.push_str(&response.content);

                if response.done {
                    break;
                }
            }
            Err(e) => {
                progress.finish_and_clear();
                return Err(format!("Stream error: {e}").into());
            }
        }
    }

    println!(); // New line after response

    // Check if the response contains @tool calls and execute them
    if let Some(mcp) = mcp_client {
        let (response_text, tool_results) =
            handle_tool_calls_in_response(&full_response, mcp, client.provider_name())?;

        // If we executed tools, display them nicely
        if !tool_results.is_empty() {
            println!(); // Extra line before tool results
            println!("=== MCP Tool Results ===");

            for (tool_name, result) in &tool_results {
                println!("\n[Tool: {tool_name}]");
                println!("Response:");

                // Pretty print the result if it's JSON
                if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(result) {
                    if let Ok(pretty) = serde_json::to_string_pretty(&json_val) {
                        println!("{pretty}");
                    } else {
                        println!("{result}");
                    }
                } else {
                    println!("{result}");
                }
            }

            println!("\n=== End Tool Results ===\n");

            // Now continue the conversation with the tool results
            // Add the tool results to the response for context
            let mut response_with_results = response_text;
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
    mcp_client: Option<&McpConnection>,
) -> Result<String, Box<dyn std::error::Error>> {
    use std::time::Instant;

    let start_time = Instant::now();
    eprintln!("[DEBUG] Starting process_message_print at {start_time:?}");
    eprintln!("[DEBUG] Messages count: {}", messages.len());
    eprintln!("[DEBUG] Provider: {}", client.provider_name());

    // Use streaming but only print content
    eprintln!("[DEBUG] Calling client.stream() to get response stream...");
    let stream_result = client.stream(messages.to_vec()).await;

    match stream_result {
        Ok(mut stream) => {
            eprintln!("[DEBUG] Stream created successfully, waiting for chunks...");
            let mut full_response = String::new();
            let mut chunk_count = 0;
            let mut total_chars = 0;

            loop {
                eprintln!(
                    "[DEBUG] Polling for next chunk (chunk #{}, elapsed: {:?})...",
                    chunk_count + 1,
                    start_time.elapsed()
                );

                match tokio::time::timeout(std::time::Duration::from_secs(30), stream.next()).await
                {
                    Ok(Some(chunk)) => {
                        chunk_count += 1;
                        eprintln!(
                            "[DEBUG] Received chunk #{} after {:?}",
                            chunk_count,
                            start_time.elapsed()
                        );

                        match chunk {
                            Ok(response) => {
                                let chunk_size = response.content.len();
                                eprintln!(
                                    "[DEBUG] Chunk #{}: {} chars, done={}",
                                    chunk_count, chunk_size, response.done
                                );

                                print!("{}", response.content);
                                io::stdout().flush()?;
                                full_response.push_str(&response.content);
                                total_chars += chunk_size;

                                if response.done {
                                    eprintln!("[DEBUG] Stream marked as done, breaking loop");
                                    break;
                                }
                            }
                            Err(e) => {
                                eprintln!("[ERROR] Stream error at chunk #{chunk_count}: {e}");
                                return Err(format!("Stream error: {e}").into());
                            }
                        }
                    }
                    Ok(None) => {
                        eprintln!("[DEBUG] Stream ended naturally after {chunk_count} chunks");
                        break;
                    }
                    Err(_) => {
                        let next_chunk = chunk_count + 1;
                        let elapsed = start_time.elapsed();
                        eprintln!(
                            "[ERROR] Timeout waiting for chunk #{next_chunk} after {elapsed:?}"
                        );
                        eprintln!(
                            "[ERROR] Received {chunk_count} chunks totaling {total_chars} chars before timeout"
                        );

                        if total_chars == 0 {
                            eprintln!("[ERROR] No response data received from model");
                            eprintln!(
                                "[ERROR] This suggests the model loaded but is not generating tokens"
                            );
                        }

                        return Err("Stream timeout: No response received within 30 seconds".into());
                    }
                }
            }

            eprintln!(
                "[DEBUG] Stream completed: {} chunks, {} total chars, elapsed: {:?}",
                chunk_count,
                total_chars,
                start_time.elapsed()
            );

            println!(); // New line at end

            // Check if the response contains @tool calls and execute them
            if let Some(mcp) = mcp_client {
                eprintln!("[DEBUG] Checking for MCP tool calls in response...");
                let (response_text, tool_results) =
                    handle_tool_calls_in_response(&full_response, mcp, client.provider_name())?;

                // If we executed tools, print them
                if !tool_results.is_empty() {
                    eprintln!("[DEBUG] Executed {} MCP tools", tool_results.len());
                    for (tool_name, result) in tool_results {
                        println!("\n[Tool Result - {tool_name}]:");

                        // Pretty print the result if it's JSON
                        if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&result) {
                            if let Ok(pretty) = serde_json::to_string_pretty(&json_val) {
                                println!("{pretty}");
                            } else {
                                println!("{result}");
                            }
                        } else {
                            println!("{result}");
                        }
                    }
                    io::stdout().flush()?;
                    return Ok(response_text);
                }
            }

            eprintln!(
                "[DEBUG] Response processing complete, total time: {:?}",
                start_time.elapsed()
            );
            Ok(full_response)
        }
        Err(e) => {
            eprintln!("[ERROR] Failed to create stream: {e}");
            eprintln!(
                "[ERROR] This may indicate a connection problem or model initialization issue"
            );
            Err(format!("Failed to create stream: {e}").into())
        }
    }
}

#[allow(dead_code)]
fn get_current_directory() -> String {
    match env::current_dir() {
        Ok(path) => path.display().to_string(),
        Err(_) => String::from("Unknown"),
    }
}

#[cfg(all(unix, feature = "test-harness"))]
#[allow(dead_code)]
fn handle_command(
    input: &str,
    mcp_client: Option<&McpConnection>,
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
                            Some(format!("Content of {file_path} (via MCP):\n\n{text}"))
                        } else {
                            Some(format!("MCP read result: {result}"))
                        }
                    }
                    Err(e) => {
                        eprintln!("MCP read failed, falling back to local: {e}");
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
                            Some(format!("Contents of {path} (via MCP):\n\n{text}"))
                        } else {
                            Some(format!("MCP list result: {result}"))
                        }
                    }
                    Err(e) => {
                        eprintln!("MCP list failed, falling back to local: {e}");
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
                            Some(format!("Available tests (via MCP):\n\n{text}"))
                        } else {
                            Some(format!("MCP test list result: {result}"))
                        }
                    }
                    Err(e) => Some(format!("Failed to list tests: {e}")),
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
                            Some(format!("Test execution result (via MCP):\n\n{text}"))
                        } else {
                            Some(format!("MCP test result: {result}"))
                        }
                    }
                    Err(e) => Some(format!("Failed to run test: {e}")),
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
                                use std::fmt::Write;
                                let _ = writeln!(output, "  {} - {}", tool.name, tool.description);
                            }
                            Some(output)
                        }
                    }
                    Err(e) => Some(format!("Failed to list MCP tools: {e}")),
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

// Type alias for tool execution results
#[allow(dead_code)]
type ToolResults = Vec<(String, String)>;

// Stub implementations when test-harness is not available
#[cfg(not(all(unix, feature = "test-harness")))]
#[allow(dead_code)]
fn handle_command(
    _input: &str,
    _mcp_client: Option<&McpConnection>,
    _llm_provider: &str,
) -> Option<String> {
    None
}

#[cfg(not(all(unix, feature = "test-harness")))]
#[allow(dead_code)]
fn handle_tool_calls_in_response(
    response: &str,
    _mcp_client: &McpConnection,
    _llm_provider: &str,
) -> Result<(String, ToolResults), Box<dyn std::error::Error>> {
    // When MCP is not available, just return the response as-is
    Ok((response.to_string(), Vec::new()))
}

#[allow(dead_code)]
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
            Some(format!("Content of {file_path}:\n\n{preview}"))
        }
        Err(e) => Some(format!("Error reading file '{file_path}': {e}")),
    }
}

#[cfg(all(unix, feature = "test-harness"))]
#[allow(dead_code)]
fn handle_tool_calls_in_response(
    response: &str,
    mcp_client: &McpConnection,
    llm_provider: &str,
) -> Result<(String, ToolResults), Box<dyn std::error::Error>> {
    // Find all @tool calls in the response
    let mut tool_results = Vec::new();

    // Return the original response text to avoid interrupting the flow
    let original_response = response.to_string();

    // Use a more robust approach to find @tool calls
    // First, remove markdown code blocks to find tools within them
    let cleaned_response = response.replace("```", "").replace('`', "");

    let remaining = &cleaned_response[..];

    // Process only the first tool call to avoid interrupting the flow
    // This allows for multi-tasking by processing one tool at a time
    if let Some(at_pos) = remaining.find('@') {
        // Check if this is a tool call (followed by word characters)
        let after_at = &remaining[at_pos + 1..];

        if let Some(space_or_brace) = after_at.find(|c: char| c.is_whitespace() || c == '{') {
            let tool_name = &after_at[..space_or_brace];

            // Only process if tool_name is alphanumeric and not exactly "screenshot" (which is not allowed)
            if tool_name.chars().all(|c| c.is_alphanumeric() || c == '_')
                && tool_name != "screenshot"
            {
                let args_start = at_pos + 1 + space_or_brace;
                let args_str = &remaining[args_start..].trim_start();

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

                        match serde_json::from_str(json_str) {
                            Ok(json) => (json, end_pos),
                            Err(_e) => (json!({ "prompt": json_str }), end_pos),
                        }
                    } else {
                        (json!({ "prompt": args_str }), 0)
                    }
                } else {
                    // Take until newline or end of string
                    let end_pos = args_str.find('\n').unwrap_or(args_str.len());
                    let arg_text = &args_str[..end_pos].trim();
                    (json!({ "prompt": arg_text }), end_pos)
                };

                // Execute the tool
                match mcp_client.call_tool(tool_name, args, llm_provider) {
                    Ok(tool_result) => {
                        // Extract the actual result text from the MCP response
                        let result_text = if let Some(result_obj) = tool_result.get("result") {
                            if let Some(text) = result_obj.as_str() {
                                text.to_string()
                            } else {
                                serde_json::to_string_pretty(&result_obj)
                                    .unwrap_or_else(|_e| result_obj.to_string())
                            }
                        } else {
                            serde_json::to_string_pretty(&tool_result)
                                .unwrap_or_else(|_e| tool_result.to_string())
                        };

                        tool_results.push((tool_name.to_string(), result_text));
                    }
                    Err(e) => {
                        tool_results.push((tool_name.to_string(), format!("Error: {e}")));
                    }
                }
            }
        }
    }

    // Return the original response text to avoid interrupting the flow
    Ok((original_response, tool_results))
}

#[allow(dead_code)]
fn list_files(path: &str) -> Option<String> {
    let path = Path::new(path);

    match fs::read_dir(path) {
        Ok(entries) => {
            let mut files = Vec::new();
            let mut dirs = Vec::new();

            for entry in entries.filter_map(std::result::Result::ok) {
                let file_name = entry.file_name().to_string_lossy().to_string();
                if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                    dirs.push(format!("{file_name}/ (dir)"));
                } else {
                    files.push(file_name);
                }
            }

            dirs.sort();
            files.sort();

            let mut result = format!("Contents of {}:\n\n", path.display());

            for dir in &dirs {
                use std::fmt::Write;
                let _ = writeln!(result, "  {dir}");
            }

            for file in &files {
                use std::fmt::Write;
                let _ = writeln!(result, "  {file}");
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

async fn initialize_llm_client(print_mode: bool) -> Result<LlmClient, Box<dyn std::error::Error>> {
    use std::time::Instant;

    let init_start = Instant::now();
    eprintln!("[DEBUG] Starting LLM client initialization, print_mode={print_mode}");

    // Initialize memory storage
    let storage = Arc::new(MemoryStorage::new().await?);

    // Check for previously selected provider
    eprintln!("[DEBUG] Checking for saved provider configuration...");
    let saved_provider = storage
        .search("llm_provider", 1, Some("llm_provider"))
        .await?
        .into_iter()
        .find(|c| c.memory.content != "CLEARED");

    if let Some(ref provider) = saved_provider {
        eprintln!("[DEBUG] Found saved provider: {}", provider.memory.content);
    } else {
        eprintln!("[DEBUG] No saved provider found");
    }

    // First priority: Try saved Ollama server if configured
    if let Some(provider_config) = &saved_provider
        && provider_config.memory.content.starts_with("http")
    {
        // Ollama server
        let server_url = &provider_config.memory.content;
        eprintln!("[DEBUG] Attempting connection to saved Ollama server: {server_url}");
        unsafe {
            std::env::set_var("OLLAMA_BASE_URL", server_url);
        }

        if let Ok(client) = LlmClient::from_env() {
            eprintln!("[DEBUG] Client created, testing connection with ping...");
            let test_message = vec![Message::user("ping")];
            let test_start = Instant::now();

            match client.complete(test_message).await {
                Ok(_) => {
                    eprintln!(
                        "[DEBUG] Connection test successful (took {:?})",
                        test_start.elapsed()
                    );
                    if !print_mode {
                        eprintln!("✓ Connected to saved Ollama server at {server_url}");
                    }
                    eprintln!(
                        "[DEBUG] Total initialization time: {:?}",
                        init_start.elapsed()
                    );
                    return Ok(client);
                }
                Err(e) => {
                    let elapsed = test_start.elapsed();
                    eprintln!("[DEBUG] Connection test failed after {elapsed:?}: {e}");
                }
            }
        } else {
            eprintln!("[DEBUG] Failed to create client from saved URL");
        }
    }

    // Second priority: Try default localhost Ollama
    eprintln!("[DEBUG] Attempting connection to localhost:11434...");
    match LlmClient::from_env() {
        Ok(client) => {
            eprintln!("[DEBUG] Local client created, testing connection...");
            // Test if the client can connect by trying a minimal request
            let test_message = vec![Message::user("ping")];
            let test_start = Instant::now();

            match client.complete(test_message).await {
                Ok(_) => {
                    eprintln!(
                        "[DEBUG] Local connection test successful (took {:?})",
                        test_start.elapsed()
                    );
                    if !print_mode {
                        eprintln!("✓ Connected to Ollama at localhost:11434");
                    }
                    eprintln!(
                        "[DEBUG] Total initialization time: {:?}",
                        init_start.elapsed()
                    );
                    return Ok(client);
                }
                Err(e) => {
                    let elapsed = test_start.elapsed();
                    eprintln!("[DEBUG] Local connection test failed after {elapsed:?}: {e}");
                    if !print_mode {
                        eprintln!("Could not connect to Ollama at localhost:11434: {e}");
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("[DEBUG] Failed to create local client: {e}");
            if !print_mode {
                eprintln!("Ollama not available: {e}");
            }
        }
    }

    // Third priority: Check if previously selected local model is still available
    if let Some(provider_config) = &saved_provider
        && provider_config.memory.content.starts_with("local:")
    {
        let model_name = provider_config
            .memory
            .content
            .strip_prefix("local:")
            .unwrap();
        eprintln!("[DEBUG] Found saved local model preference: {model_name}");
        if !print_mode {
            eprintln!("Checking for previously used local model: {model_name}");
        }
    }

    // Fourth priority: Try local models from HuggingFace cache
    #[cfg(feature = "local")]
    {
        use arkavo_llm::local::{ModelDownloader, ModelManifest};

        eprintln!("[DEBUG] Checking for local models in HuggingFace cache...");
        if !print_mode {
            eprintln!("Checking for local models in HuggingFace cache...");
        }

        // Load manifest and try models in priority order
        match ModelManifest::load() {
            Ok(manifest) => {
                eprintln!("[DEBUG] Model manifest loaded successfully");

                // Priority order: Phi-2 first (since it's the one mentioned in the issue)
                let model_priorities = [
                    "phi-2-q4k",          // Phi-2 as primary
                    "tinyllama-110m-f16", // Smallest model for testing
                    "gemma3-1b-it-qat",   // Gemma 3 1B
                    "tinyllama-1b-chat-q2",
                    "tinyllama-1b-chat-q3",
                    "tinyllama-1b-chat",
                    "gemma3n-e4b-it", // Gemma 3n E4B - not yet supported by Candle
                ];

                eprintln!("[DEBUG] Trying models in priority order: {model_priorities:?}");

                for model_name in &model_priorities {
                    eprintln!("[DEBUG] Checking for model: {model_name}");

                    if let Some(spec) = manifest.find(model_name) {
                        eprintln!("[DEBUG] Found model spec for: {model_name}");

                        // Create downloader to check cache
                        match ModelDownloader::new() {
                            Ok(downloader) => {
                                eprintln!("[DEBUG] Model downloader created, checking cache...");

                                // This will return cached path if already downloaded
                                match downloader.get_model_path(spec).await {
                                    Ok(model_path) => {
                                        eprintln!(
                                            "[DEBUG] Model found in cache at: {}",
                                            model_path.display()
                                        );

                                        if !print_mode {
                                            eprintln!(
                                                "Found cached model: {} at {}",
                                                spec.name,
                                                model_path.display()
                                            );
                                        }

                                        eprintln!(
                                            "[DEBUG] Initializing local model: {}",
                                            spec.name
                                        );
                                        let load_start = Instant::now();

                                        match LlmClient::from_local_model(
                                            &spec.name,
                                            model_path.to_string_lossy().to_string(),
                                        )
                                        .await
                                        {
                                            Ok(client) => {
                                                eprintln!(
                                                    "[DEBUG] Model loaded successfully in {:?}",
                                                    load_start.elapsed()
                                                );
                                                if !print_mode {
                                                    eprintln!("✓ Using local model: {}", spec.name);
                                                }
                                                eprintln!(
                                                    "[DEBUG] Total initialization time: {:?}",
                                                    init_start.elapsed()
                                                );
                                                return Ok(client);
                                            }
                                            Err(e) => {
                                                eprintln!(
                                                    "[ERROR] Failed to initialize model {} after {:?}: {}",
                                                    spec.name,
                                                    load_start.elapsed(),
                                                    e
                                                );
                                                if !print_mode {
                                                    eprintln!(
                                                        "Failed to initialize local model {}: {}",
                                                        spec.name, e
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("[DEBUG] Model {model_name} not in cache: {e}");
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("[ERROR] Failed to create model downloader: {e}");
                            }
                        }
                    } else {
                        eprintln!("[DEBUG] Model {model_name} not found in manifest");
                    }
                }
            }
            Err(e) => {
                eprintln!("[ERROR] Failed to load model manifest: {e}");
            }
        }
    }

    // Last resort: Prompt for remote Ollama server
    prompt_for_remote_ollama(print_mode, storage).await
}

async fn prompt_for_remote_ollama(
    print_mode: bool,
    storage: Arc<MemoryStorage>,
) -> Result<LlmClient, Box<dyn std::error::Error>> {
    if print_mode {
        return Err(
            "Ollama is not running locally and print mode doesn't support interactive prompts"
                .into(),
        );
    }

    eprintln!("⚠️  Could not connect to Ollama at localhost:11434");
    eprintln!("Please ensure Ollama is running or provide a remote server address.");
    eprintln!();
    eprintln!("Tip: To clear saved configuration, type 'clear'");
    print!("Enter Ollama server address (e.g., 192.168.1.100:11434) or press Enter to cancel: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if input.is_empty() {
        return Err("No Ollama server configured".into());
    }

    // Check if user wants to clear saved configuration
    if input == "clear" {
        // Delete saved configuration by searching and marking as deleted
        let saved_configs = storage
            .search("ollama_server_config", 100, Some("config"))
            .await?;
        for config in saved_configs {
            // We can't delete, but we can update the content to mark it as cleared
            let mut cleared_memory = config.memory;
            cleared_memory.content = "CLEARED".to_string();
            cleared_memory.updated_at = chrono::Utc::now();
            if let Err(e) = storage.store(cleared_memory).await {
                eprintln!("Warning: Could not clear configuration: {e}");
            }
        }
        eprintln!("✓ Cleared saved Ollama server configuration");
        return Err("Please restart the command to configure a new server".into());
    }

    // Ensure the URL has the correct format
    let base_url = if input.starts_with("http://") || input.starts_with("https://") {
        input.to_string()
    } else {
        format!("http://{input}")
    };

    // Set the environment variable for this session
    unsafe {
        std::env::set_var("OLLAMA_BASE_URL", &base_url);
    }

    // Try to create client with the new URL and test it
    match LlmClient::from_env() {
        Ok(client) => {
            // Test connection with a minimal request
            let test_message = vec![Message::user("ping")];
            match client.complete(test_message).await {
                Ok(_) => {
                    eprintln!("✓ Connected to Ollama at {base_url}");

                    // Save the configuration for future use
                    // Generate a dummy embedding since we're not using embeddings feature
                    let embedding_service = arkavo_memory::embeddings::EmbeddingService::new();
                    let embedding = match embedding_service.generate_embedding(&base_url).await {
                        Ok(e) => e,
                        Err(_) => vec![0.0; 384], // Default embedding size
                    };

                    let memory = arkavo_memory::models::Memory {
                        id: uuid::Uuid::new_v4(),
                        content: base_url.clone(),
                        metadata: Some(json!({
                            "type": "ollama_server_config",
                            "timestamp": chrono::Utc::now().to_rfc3339()
                        })),
                        category: Some("config".to_string()),
                        embedding,
                        created_at: chrono::Utc::now(),
                        updated_at: chrono::Utc::now(),
                    };

                    if let Err(e) = storage.store(memory).await {
                        eprintln!("Warning: Could not save Ollama server configuration: {e}");
                    } else {
                        eprintln!("✓ Saved configuration for future use");
                    }

                    Ok(client)
                }
                Err(e) => Err(format!("Failed to connect to Ollama at {base_url}: {e}").into()),
            }
        }
        Err(e) => Err(format!("Failed to create client for {base_url}: {e}").into()),
    }
}

#[allow(clippy::disallowed_methods)]
fn launch_terminal_ui(runtime: Runtime) -> Result<(), Box<dyn std::error::Error>> {
    // For TUI mode, we bypass all the initialization and go straight to the UI
    // The UI will handle its own initialization
    runtime.block_on(async { arkavo_terminal::run().await })?;
    Ok(())
}
