use crate::conversation_manager::ConversationManager;
use crate::mcp_integration::McpConnection;
use arkavo_llm::{LlmClient, Message, encode_image_file};
use arkavo_memory::storage::MemoryStorage;
use arkavo_repo::repository_context::RepositoryContextManager;
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

// Global flag to control whether to show debug messages
// Set via ARKAVO_DEBUG_CHAT=1 environment variable
static SHOW_DEBUG: AtomicBool = AtomicBool::new(false);

// Global flag for repo context mode (can be changed via REPL command)
static REPO_CONTEXT_MODE: std::sync::RwLock<String> = std::sync::RwLock::new(String::new());

// Global runtime to prevent multiple runtime creation issues
static RUNTIME: std::sync::OnceLock<Runtime> = std::sync::OnceLock::new();

fn get_or_create_runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("arkavo-chat-worker")
            .thread_stack_size(3 * 1024 * 1024)
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime")
    })
}

/// Determines if repository context should be attached based on the prompt
fn should_attach_repo_context(prompt: &str) -> bool {
    let p = prompt.trim();

    // Short math/calculation pattern - no context needed
    if p.len() < 32 {
        // Check if it's just numbers and math operators
        let is_math = p.chars().all(|c| "0123456789+-*/().=? \t\n".contains(c));
        if is_math {
            if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
                eprintln!("[DEBUG] RepoCtx: skipped (short math expression)");
            }
            return false;
        }
    }

    // Greeting patterns - no context needed
    let greetings = [
        "hi",
        "hello",
        "hey",
        "good morning",
        "good afternoon",
        "good evening",
    ];
    let lower = p.to_lowercase();
    if p.len() < 20
        && greetings
            .iter()
            .any(|g| lower == *g || lower == format!("{g}!"))
    {
        if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!("[DEBUG] RepoCtx: skipped (greeting)");
        }
        return false;
    }

    // Keywords that indicate code/doc related queries - context needed
    let code_keywords = [
        "readme",
        "build",
        "compile",
        "cargo",
        "rust",
        "error:",
        "panic",
        ".rs",
        ".gd",
        ".swift",
        ".toml",
        "arkavo",
        "how do i",
        "how to",
        "api",
        "design",
        "function",
        "struct",
        "impl",
        "trait",
        "module",
        "test",
        "debug",
        "fix",
        "implement",
        "code",
        "file",
        "directory",
        "git",
        "branch",
        "commit",
        "repository",
        "project",
        "crate",
    ];

    if code_keywords.iter().any(|k| lower.contains(k)) {
        if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!("[DEBUG] RepoCtx: will inject (code/doc keyword detected)");
        }
        return true;
    }

    // Default: no context for unrecognized patterns
    if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
        eprintln!("[DEBUG] RepoCtx: skipped (no relevant keywords)");
    }
    false
}

/// Compresses repository context to a digest suitable for small models
fn compress_repo_context(full_content: &str, max_tokens: usize) -> String {
    // For now, take the first part of the content with a clear header
    // In the future, this could do smart summarization
    let header = "[PROJECT CONTEXT DIGEST — KEEP ANSWER CONCISE]\n";

    // Estimate ~4 chars per token (rough approximation)
    let max_chars = max_tokens * 4;

    if full_content.len() <= max_chars {
        format!("{header}{full_content}")
    } else {
        // Take key sections: title, overview, and first few command examples
        let lines: Vec<&str> = full_content.lines().collect();
        let mut digest = String::from(header);
        let mut token_budget = max_tokens - 20; // Reserve some for header

        for line in lines {
            // Prioritize headers and overview sections
            if line.starts_with("# ")
                || line.starts_with("## Project Overview")
                || line.starts_with("## Key Command")
                || line.contains("arkavo")
            {
                digest.push_str(line);
                digest.push('\n');
                token_budget = token_budget.saturating_sub(line.len() / 4);
                if token_budget < 50 {
                    break;
                }
            }
        }

        digest.push_str("\n(Use 'show docs' for more details)\n");
        digest
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

/// Execute the chat command
///
/// # Panics
///
/// May panic if RwLock is poisoned
#[allow(clippy::disallowed_methods)]
#[allow(clippy::significant_drop_tightening)]
pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // Check for --debug flag in arguments
    if args.iter().any(|arg| arg == "--debug") {
        SHOW_DEBUG.store(true, std::sync::atomic::Ordering::Relaxed);
    }

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

    // Check if --new-session flag is present to start fresh without history
    let new_session = args.contains(&"--new-session".to_string());

    // Parse model selection
    let model_name = args
        .windows(2)
        .find(|w| w[0] == "--model")
        .map(|w| w[1].clone())
        .unwrap_or_else(|| "gemma-3-270m".to_string());

    // Parse repo context mode: auto (default), on, off
    let repo_context_mode = args
        .windows(2)
        .find(|w| w[0] == "--repo-context")
        .map(|w| w[1].clone())
        .or_else(|| env::var("ARKAVO_REPO_CONTEXT").ok())
        .unwrap_or_else(|| "auto".to_string());

    // Store repo context mode globally for REPL commands
    {
        let mut mode = REPO_CONTEXT_MODE.write().unwrap();
        *mode = repo_context_mode;
    }

    // Parse sampling parameters for llama.cpp
    let temperature = args
        .windows(2)
        .find(|w| w[0] == "--temperature")
        .and_then(|w| w[1].parse::<f32>().ok())
        .unwrap_or(0.7);
    let top_p = args
        .windows(2)
        .find(|w| w[0] == "--top-p")
        .and_then(|w| w[1].parse::<f32>().ok())
        .unwrap_or(0.9);
    let top_k = args
        .windows(2)
        .find(|w| w[0] == "--top-k")
        .and_then(|w| w[1].parse::<i32>().ok())
        .unwrap_or(40);
    let max_tokens = args
        .windows(2)
        .find(|w| w[0] == "--max-tokens")
        .and_then(|w| w[1].parse::<u32>().ok())
        .or_else(|| {
            std::env::var("ARKAVO_MAX_TOKENS")
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
        })
        .unwrap_or(4096); // Default to 4K tokens
    let seed = args
        .windows(2)
        .find(|w| w[0] == "--seed")
        .and_then(|w| w[1].parse::<u32>().ok())
        .unwrap_or_else(|| {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            std::time::SystemTime::now().hash(&mut hasher);
            hasher.finish() as u32
        });

    // Use global runtime to prevent multiple runtime creation issues
    let runtime = get_or_create_runtime();

    // Initialize memory storage
    let memory_storage = Arc::new(runtime.block_on(MemoryStorage::new())?);

    // Initialize conversation manager
    let mut conversation_manager = ConversationManager::new(memory_storage.clone())?;

    // Initialize repository context manager
    let _repo_context_manager = RepositoryContextManager::new(memory_storage)?;

    // Initialize LLM client with fallback to prompt for remote server
    let client = runtime.block_on(initialize_llm_client(
        print_mode,
        &model_name,
        temperature,
        top_p,
        top_k,
        max_tokens,
        seed,
    ))?;

    // Detect model size hint from model name
    let model_size_hint =
        if client.provider_name().contains("270m") || client.provider_name().contains("270M") {
            Some("270M")
        } else if client.provider_name().contains("1b") || client.provider_name().contains("1B") {
            Some("1B")
        } else if client.provider_name().contains("2b") || client.provider_name().contains("2B") {
            Some("2B")
        } else if client.provider_name().contains("7b") || client.provider_name().contains("7B") {
            Some("7B")
        } else {
            None
        };

    if !print_mode {
        println!("Starting UI testing chat session...");
        println!("Repository context: {}", get_current_directory());
        println!("LLM Provider: {}", client.provider_name());

        // Show model-specific settings for small models
        if let Some(size) = &model_size_hint {
            println!("Model size: {size} - using limited history");
        }

        // Try to restore last session unless --new-session is specified
        if new_session {
            // Start fresh session with metadata
            let _ = runtime.block_on(conversation_manager.start_session_with_metadata(
                client.provider_name(),
                Some("gemma3"), // TODO: Get actual template name
                None,           // System prompt not available yet
                model_size_hint,
            ))?;
            println!("Started new conversation session");
        } else if let Ok(Some(session_id)) = runtime.block_on(
            conversation_manager.restore_last_session_with_compatibility(
                Some("gemma3"), // TODO: Get actual template name
                None,           // System prompt not available yet
                Some(client.provider_name()),
            ),
        ) {
            println!(
                "Restored previous conversation (session: {})",
                &session_id.to_string()[..8]
            );
        } else {
            // Start new session with metadata
            let _ = runtime.block_on(conversation_manager.start_session_with_metadata(
                client.provider_name(),
                Some("gemma3"), // TODO: Get actual template name
                None,           // System prompt not available yet
                model_size_hint,
            ))?;
        }

        println!("Type '/exit' or '/quit' to end the session.");
        println!("Commands: /read <file>, /list [path], /test, /run <test_name>, /tools");
        println!(
            "Session: /new (start fresh), /clear (clear history), /history (show stats), /switch <id>"
        );
        println!("Context: /context {{auto|on|off}} - Control repository context injection");
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
    let _repo_context = json!({
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

    // Build base system prompt (without repo context initially)
    let base_system_prompt = if mcp_client.is_some() {
        format!(
            "You are a helpful AI assistant with access to MCP tools for development tasks.\
            \n{mcp_info}"
        )
    } else {
        "You are a helpful AI assistant. Be concise and direct in your responses.".to_string()
    };

    // Determine whether to inject repo context
    let should_inject_context = if let (true, Some(ref p)) = (print_mode, prompt.as_ref()) {
        // For print mode with prompt, check if we should attach context
        let current_mode = REPO_CONTEXT_MODE.read().unwrap();
        match (model_size_hint, current_mode.as_str()) {
            // Tiny models: only inject if explicitly on or auto with relevant query
            (Some("270M" | "1B"), "on") => true,
            (Some("270M" | "1B"), "auto") => should_attach_repo_context(p),
            (Some("270M" | "1B"), _) => false,
            // Larger models: inject unless off or auto with irrelevant query
            (_, "off") => false,
            (_, "auto") => should_attach_repo_context(p),
            (_, _) => true,
        }
    } else if !print_mode {
        // Interactive mode: check based on model size and mode
        let current_mode = REPO_CONTEXT_MODE.read().unwrap();
        match (model_size_hint, current_mode.as_str()) {
            (Some("270M" | "1B"), "on") => true,
            (Some("270M" | "1B"), _) => false, // Default off for tiny in interactive
            (_, "off") => false,
            _ => true, // Default on for larger models in interactive
        }
    } else {
        false // No context for other cases
    };

    let system_prompt = if should_inject_context {
        // Try to read AGENTS.md or CLAUDE.md
        let agents_md_content = match std::fs::read_to_string("AGENTS.md") {
            Ok(content) => Some(content),
            Err(_) => std::fs::read_to_string("CLAUDE.md").ok(),
        };

        if let Some(agents_content) = agents_md_content {
            // Determine context size based on model
            let max_context_tokens = match model_size_hint {
                Some("270M") => 200,
                Some("1B") => 300,
                Some("2B") => 500,
                _ => 1000,
            };

            let compressed_context = compress_repo_context(&agents_content, max_context_tokens);

            if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
                let token_estimate = compressed_context.len() / 4;
                eprintln!("[DEBUG] RepoCtx: injected={token_estimate} tokens (compressed)");
            }

            format!(
                "{base_system_prompt}\n\n{compressed_context}\n\nWorking directory: {repo_context_str}"
            )
        } else {
            // No README found, use base prompt with minimal context
            format!("{base_system_prompt}\n\nWorking directory: {repo_context_str}")
        }
    } else {
        // No context injection
        if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!("[DEBUG] RepoCtx: not injected (model/mode policy)");
        }
        base_system_prompt
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
        // In interactive mode, get context with appropriate history limits
        let history_limit = if model_size_hint == Some("270M") {
            Some(2) // Very limited for tiny models
        } else if let Some("1B" | "2B") = model_size_hint {
            Some(4)
        } else {
            None // Use default
        };

        runtime.block_on(
            conversation_manager
                .get_context_messages_with_limits(Some(system_message.clone()), history_limit),
        )?
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

        // Handle /new command - start fresh session
        if input == "/new" {
            let _ = runtime.block_on(conversation_manager.start_session(client.provider_name()))?;
            messages = vec![system_message.clone()];
            println!("Started new conversation session");
            continue;
        }

        // Handle /clear command - clear current session
        if input == "/clear" {
            conversation_manager.clear_session()?;
            let _ = runtime.block_on(conversation_manager.start_session(client.provider_name()))?;
            messages = vec![system_message.clone()];
            println!("Cleared conversation history");
            continue;
        }

        // Handle /context command - toggle repo context mode
        if input.starts_with("/context") {
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() > 1 {
                let new_mode = parts[1];
                if ["auto", "on", "off"].contains(&new_mode) {
                    {
                        let mut mode = REPO_CONTEXT_MODE.write().unwrap();
                        *mode = new_mode.to_string();
                    }
                    println!("Repository context mode set to: {new_mode}");
                    match new_mode {
                        "auto" => println!("  Context will be injected based on query relevance"),
                        "on" => println!("  Context will always be injected"),
                        "off" => println!("  Context will never be injected"),
                        _ => {}
                    }
                } else {
                    println!("Invalid mode. Use: /context {{auto|on|off}}");
                }
            } else {
                {
                    let mode = REPO_CONTEXT_MODE.read().unwrap();
                    println!("Current repository context mode: {mode}");
                }
                println!("Use: /context {{auto|on|off}} to change");
            }
            continue;
        }

        // Handle /history command - show session stats
        if input.starts_with("/history") {
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() > 1 && parts[1] == "--last" {
                // Show last N messages
                let n = parts
                    .get(2)
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(5);
                println!("\nLast {n} messages:");
                let display_messages = messages.iter().rev().take(n).rev();
                for msg in display_messages {
                    match msg.role {
                        arkavo_llm::Role::User => println!("User: {}", msg.content),
                        arkavo_llm::Role::Assistant => {
                            let preview = if msg.content.len() > 100 {
                                format!("{}...", &msg.content[..100])
                            } else {
                                msg.content.clone()
                            };
                            println!("Assistant: {preview}");
                        }
                        _ => {}
                    }
                }
            } else {
                // Show session statistics
                match runtime.block_on(conversation_manager.get_session_stats()) {
                    Ok((turns, tokens, fence_balanced)) => {
                        println!("\nSession statistics:");
                        println!("  Messages: {turns}");
                        println!("  Tokens: {tokens}");
                        println!(
                            "  Code fence parity: {}",
                            if fence_balanced {
                                "✓ balanced"
                            } else {
                                "⚠ UNBALANCED"
                            }
                        );
                        if !fence_balanced {
                            println!(
                                "  Warning: Unbalanced code fences may cause generation issues"
                            );
                        }
                    }
                    Err(e) => eprintln!("Error getting session stats: {e}"),
                }
            }
            continue;
        }

        if input == "clear" {
            // Legacy clear command - redirect to /clear
            println!("Use /clear to clear conversation history");
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
    // Debug output if enabled
    if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
        eprintln!(
            "[DEBUG] Processing message with {} messages",
            messages.len()
        );

        // Show template info (first 200 and last 200 chars)
        let full_context = messages
            .iter()
            .map(|m| format!("{:?}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");

        let preview = if full_context.len() > 400 {
            format!(
                "{}...{}",
                &full_context[..200],
                &full_context[full_context.len() - 200..]
            )
        } else {
            full_context.clone()
        };
        eprintln!("[DEBUG] Context preview: {preview}");

        // Check fence parity
        let fence_count = full_context.matches("```").count();
        eprintln!(
            "[DEBUG] Fence parity check: {} fences ({})",
            fence_count,
            if fence_count % 2 == 0 {
                "balanced"
            } else {
                "UNBALANCED!"
            }
        );
    }

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
    if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
        eprintln!("[DEBUG] Starting process_message_print at {start_time:?}");
        eprintln!("[DEBUG] Messages count: {}", messages.len());
        eprintln!("[DEBUG] Provider: {}", client.provider_name());

        // Show template info
        let full_context = messages
            .iter()
            .map(|m| format!("{:?}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");

        let preview = if full_context.len() > 400 {
            format!(
                "{}...{}",
                &full_context[..200],
                &full_context[full_context.len() - 200..]
            )
        } else {
            full_context.clone()
        };
        eprintln!("[DEBUG] Context preview: {preview}");

        // Check fence parity
        let fence_count = full_context.matches("```").count();
        eprintln!(
            "[DEBUG] Fence parity: {} ({})",
            fence_count,
            if fence_count % 2 == 0 {
                "balanced"
            } else {
                "UNBALANCED!"
            }
        );
    }

    // Use streaming but only print content
    if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
        eprintln!("[DEBUG] Calling client.stream() to get response stream...");
    }
    let stream_result = client.stream(messages.to_vec()).await;

    match stream_result {
        Ok(mut stream) => {
            if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
                eprintln!("[DEBUG] Stream created successfully, waiting for chunks...");
            }
            let mut full_response = String::new();
            let mut chunk_count = 0;
            let mut total_chars = 0;

            loop {
                if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
                    eprintln!(
                        "[DEBUG] Polling for next chunk (chunk #{}, elapsed: {:?})...",
                        chunk_count + 1,
                        start_time.elapsed()
                    );
                }

                match tokio::time::timeout(std::time::Duration::from_secs(30), stream.next()).await
                {
                    Ok(Some(chunk)) => {
                        chunk_count += 1;
                        if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
                            eprintln!(
                                "[DEBUG] Received chunk #{} after {:?}",
                                chunk_count,
                                start_time.elapsed()
                            );
                        }

                        match chunk {
                            Ok(response) => {
                                let chunk_size = response.content.len();
                                if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
                                    eprintln!(
                                        "[DEBUG] Chunk #{}: {} chars, done={}",
                                        chunk_count, chunk_size, response.done
                                    );
                                }

                                print!("{}", response.content);
                                io::stdout().flush()?;
                                full_response.push_str(&response.content);
                                total_chars += chunk_size;

                                if response.done {
                                    if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
                                        eprintln!("[DEBUG] Stream marked as done, breaking loop");
                                    }
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
                        if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
                            eprintln!("[DEBUG] Stream ended naturally after {chunk_count} chunks");
                        }
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

            if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
                eprintln!(
                    "[DEBUG] Stream completed: {} chunks, {} total chars, elapsed: {:?}",
                    chunk_count,
                    total_chars,
                    start_time.elapsed()
                );
            }

            println!(); // New line at end

            // Check if the response contains @tool calls and execute them
            if let Some(mcp) = mcp_client {
                if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
                    eprintln!("[DEBUG] Checking for MCP tool calls in response...");
                }
                let (response_text, tool_results) =
                    handle_tool_calls_in_response(&full_response, mcp, client.provider_name())?;

                // If we executed tools, print them
                if !tool_results.is_empty() {
                    if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
                        eprintln!("[DEBUG] Executed {} MCP tools", tool_results.len());
                    }
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

            if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
                eprintln!(
                    "[DEBUG] Response processing complete, total time: {:?}",
                    start_time.elapsed()
                );
            }
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

async fn initialize_llm_client(
    print_mode: bool,
    model_name: &str,
    temperature: f32,
    top_p: f32,
    top_k: i32,
    max_tokens: u32,
    seed: u32,
) -> Result<LlmClient, Box<dyn std::error::Error>> {
    // Try to connect to Ollama first
    if let Ok(client) = LlmClient::from_env()
        && client.complete(vec![Message::user("ping")]).await.is_ok()
    {
        if !print_mode {
            println!("✓ Connected to Ollama at {}", client.provider_name());
        }
        return Ok(client);
    }

    if !print_mode {
        println!("Ollama not available.");
    }

    // Fallback to local llama.cpp
    #[cfg(feature = "llama-cpp")]
    {
        use hf_hub::api::tokio::Api;
        use std::io::{self, Write};

        if !print_mode {
            println!("Attempting to use local model with llama.cpp...");
        }

        // Select model based on parameter
        let (model_repo, model_file, display_name) = match model_name {
            "gemma-2-2b" | "gemma-2b" => (
                "bartowski/gemma-2-2b-it-GGUF",
                "gemma-2-2b-it-Q4_K_M.gguf",
                "gemma-2-2b-it",
            ),
            _ => (
                // Default to gemma-3-270m
                "unsloth/gemma-3-270m-it-GGUF",
                "gemma-3-270m-it-Q4_0.gguf",
                "gemma-3-270m-it",
            ),
        };

        if !print_mode {
            println!("Loading model: {display_name}");
        }

        let api = Api::new()?;
        let repo = api.repo(hf_hub::Repo::model(model_repo.to_string()));
        let model_path = repo.get(model_file).await;

        let final_path = match model_path {
            Ok(path) => path,
            Err(_) => {
                if !print_mode {
                    print!("Model '{model_file}' not found locally. Download now? (Y/n) ");
                    io::stdout().flush()?;
                    let mut input = String::new();
                    io::stdin().read_line(&mut input)?;
                    if input.trim().to_lowercase() != "y" && !input.trim().is_empty() {
                        return Err("Download declined by user.".into());
                    }
                }
                println!("Downloading model... this may take a while.");
                let download_path = repo.get(model_file).await?;
                println!("Model downloaded to: {}", download_path.display());
                download_path
            }
        };
        LlmClient::from_llamacpp_model_with_config(
            display_name,
            final_path.to_string_lossy().to_string(),
            temperature,
            top_p,
            top_k,
            max_tokens,
            seed,
        )
        .await
        .map_err(|e| e.into())
    }

    #[cfg(not(feature = "llama-cpp"))]
    {
        Err(
            "No LLM provider available. Please install Ollama or enable the 'llama-cpp' feature."
                .into(),
        )
    }
}
