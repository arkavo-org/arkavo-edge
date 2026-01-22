use crate::commands::agent::extract_agent_role;
use crate::conversation_manager::ConversationManager;
use crate::mcp_integration::McpConnection;
use arkavo_llm::{LlmClient, LlmConfig, Message, encode_image_file};
use arkavo_memory::{ContextLedger, storage::MemoryStorage};
#[cfg(not(all(unix, feature = "mcp-tools")))]
use arkavo_protocol::server::{RlmBridge, estimate_tokens, model_context_size};
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
#[cfg(not(all(unix, feature = "mcp-tools")))]
use tokio_stream::StreamExt;
use uuid;

// Global flag to control whether to show debug messages
// Set via --debug command line flag
pub(crate) static SHOW_DEBUG: AtomicBool = AtomicBool::new(false);

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

/// Check if RLM mode should activate and return system prompt if so
#[cfg(not(all(unix, feature = "mcp-tools")))]
fn check_rlm_activation(content: &str, model_hint: Option<&str>, is_cloud: bool) -> Option<String> {
    let input_tokens = estimate_tokens(content);
    let context_size = model_context_size(model_hint, is_cloud);
    let rlm_bridge = RlmBridge::with_default_manager();

    if rlm_bridge.should_activate(input_tokens, context_size) {
        if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!(
                "[DEBUG] RLM mode would activate: {} tokens > {}% of {} context window",
                input_tokens, 70, context_size
            );
        }
        // Return a hint that RLM mode should be used
        // Full RLM decomposition requires async context; this is a detection hint
        Some(format!(
            "Note: Input context ({} tokens) exceeds 70% of model context ({} tokens). \
             Consider using 'arkavo task' with mesh agents for optimal RLM processing.",
            input_tokens, context_size
        ))
    } else {
        None
    }
}

/// Strip LLM internal blocks (`<think>`, `<tool>`, conversation prefixes) for clean output
/// In debug mode, prints thinking content to stderr before stripping
fn strip_think_blocks(content: &str) -> String {
    let mut result = content.to_string();
    let debug = SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed);

    // Extract and optionally display think blocks
    while let Some(start) = result.find("<think>") {
        if let Some(end) = result.find("</think>") {
            let think_content = &result[start + 7..end];
            if debug && !think_content.trim().is_empty() {
                eprintln!("[thinking] {}", think_content.trim());
            }
            let end_pos = end + "</think>".len();
            result = format!("{}{}", &result[..start], &result[end_pos..]);
        } else {
            // Unclosed think block - remove from <think> to end
            if debug {
                let think_content = &result[start + 7..];
                if !think_content.trim().is_empty() {
                    eprintln!("[thinking] {}", think_content.trim());
                }
            }
            result = result[..start].to_string();
            break;
        }
    }

    // Strip <tool>...</tool> blocks (already executed)
    while let Some(start) = result.find("<tool>") {
        if let Some(end) = result.find("</tool>") {
            let end_pos = end + "</tool>".len();
            result = format!("{}{}", &result[..start], &result[end_pos..]);
        } else {
            result = result[..start].to_string();
            break;
        }
    }

    // Strip conversation prefixes that shouldn't be in output
    for prefix in ["Human:", "Assistant:", "User:", "A:", "Q:"] {
        result = result.replace(prefix, "");
    }

    result
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

// Runtime MCP initialization - checks if mcp-tools feature is available
fn initialize_mcp_connection(print_mode: bool) -> Option<McpConnection> {
    // Try platform-specific MCP first
    #[cfg(all(target_os = "macos", feature = "mcp-tools"))]
    {
        // Try in-process MCP first on macOS, which includes iOS simulator tools
        let result = McpConnection::new_in_process();
        match result {
            Ok(client) => {
                if !print_mode {
                    eprintln!("✓ Using macOS in-process MCP server (with iOS tools)");
                }
                return Some(client);
            }
            Err(_e) => {
                // Fall through to try cross-platform tools
            }
        }
    }

    // Try cross-platform tools on Unix systems
    #[cfg(all(unix, feature = "mcp-tools"))]
    {
        let result = McpConnection::new_cross_platform();
        match result {
            Ok(client) => {
                if !print_mode {
                    eprintln!("✓ Using cross-platform MCP tools (git, filesystem, code analysis)");
                }
                return Some(client);
            }
            Err(_e) => {
                // Fall through to try external connection
            }
        }
    }

    // Finally, try external MCP connection
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
                eprintln!("ℹ MCP tools not available - using LLM-only mode");
                eprintln!(
                    "  To enable git/filesystem tools, rebuild with: cargo build --features mcp-tools"
                );
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
    // Check for --help flag first
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("Start interactive chat with LLM\n");
        println!("USAGE:");
        println!("    arkavo chat                     Interactive chat mode");
        println!("    arkavo chat --prompt \"query\"    One-shot query\n");
        println!("EXAMPLES:");
        println!("    arkavo chat");
        println!("    arkavo chat --prompt \"What is 2+2?\"\n");
        println!("OPTIONS:");
        println!("    --prompt <TEXT>       One-shot query (exits after response)");
        println!("    --image <PATH>        Attach image for vision queries");
        println!("    --model <NAME>        Model to use (default: qwen3)");
        println!("    --repo-context MODE   auto|on|off (default: auto)");
        println!("    --new-session         Start fresh without history");
        println!("    --debug               Show debug output");
        println!("    --temperature <N>     Sampling temperature (default: 0.7)");
        println!("    --top-p <N>           Top-p sampling (default: 0.9)");
        println!("    --top-k <N>           Top-k sampling (default: 40)");
        println!("    --max-tokens <N>      Max tokens (default: 4096)");
        println!("    --seed <N>            Random seed");
        println!("    -h, --help            Show this help\n");
        println!("INTERACTIVE COMMANDS:");
        println!("    /new                  Start fresh session");
        println!("    /clear                Clear session history");
        println!("    /context {{auto|on|off}}  Control repo context");
        println!("    /history              Show session stats");
        println!("    /switch <id>          Switch sessions");
        println!("    /read <file>          Read file");
        println!("    /list [path]          List directory");
        println!("    /exit, /quit          Exit chat");
        return Ok(());
    }

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
        .unwrap_or_else(|| "qwen3".to_string());

    // Parse repo context mode: auto (default), on, off
    let repo_context_mode = args
        .windows(2)
        .find(|w| w[0] == "--repo-context")
        .map(|w| w[1].clone())
        .or_else(|| env::var("ARKAVO_REPO_CONTEXT").ok())
        .unwrap_or_else(|| "auto".to_string());

    // Store repo context mode globally for REPL commands
    {
        let mut mode = REPO_CONTEXT_MODE.write().unwrap_or_else(|e| e.into_inner());
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

    // Initialize context ledger for archiving old messages (shares storage with conversation manager)
    let context_ledger = ContextLedger::new(memory_storage.clone());

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
    let provider_name = client.provider_name().to_lowercase();
    let model_size_hint = if provider_name.contains("270m") {
        Some("270M")
    } else if provider_name.contains("1b") {
        Some("1B")
    } else if provider_name.contains("2b") {
        Some("2B")
    } else if provider_name.contains("7b") {
        Some("7B")
    } else {
        None
    };

    // Detect cloud providers with large context windows
    let is_large_context_provider = provider_name.contains("gemini")
        || provider_name.contains("claude")
        || provider_name.contains("gpt")
        || provider_name.contains("deepseek")
        || provider_name.contains("anthropic")
        || provider_name.contains("openai");

    // Context offload threshold scales with model context window
    // Small models: offload early to preserve working memory
    // Cloud models: much larger context windows (Gemini up to 1M tokens)
    let (context_offload_threshold, preserve_recent_messages) = if is_large_context_provider {
        (262_144, 20) // ~64K tokens for cloud models, preserve 20 turns
    } else {
        match model_size_hint {
            Some("270M") => (2_048, 2),    // ~512 tokens, preserve 2 turns
            Some("1B") => (4_096, 4),      // ~1K tokens, preserve 4 turns
            Some("2B") => (8_192, 4),      // ~2K tokens, preserve 4 turns
            Some("7B") => (16_384, 6),     // ~4K tokens, preserve 6 turns
            Some(_) | None => (32_768, 8), // ~8K tokens for large local models
        }
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
                Some("gemma3"), // Default template for local models
                None,           // System prompt not available yet
                model_size_hint,
            ))?;
            println!("Started new conversation session");
        } else if let Ok(Some(session_id)) = runtime.block_on(
            conversation_manager.restore_last_session_with_compatibility(
                Some("gemma3"), // Default template for local models
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
                Some("gemma3"), // Default template for local models
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

    // Initialize MCP client - always try to initialize for tool access
    #[allow(unused_variables)]
    let mcp_client: Option<McpConnection> = initialize_mcp_connection(print_mode);

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

    // Debug: Show MCP tools being passed to LLM
    if std::env::var("ARKAVO_DEBUG_CHAT").is_ok() {
        eprintln!("\n=== MCP TOOLS DEBUG ===");
        if let Some(ref client) = mcp_client {
            match client.list_tools() {
                Ok(tools) => {
                    eprintln!("Tools registered: {}", tools.len());
                    for tool in &tools {
                        eprintln!("  - {} : {}", tool.name, tool.description);
                    }
                }
                Err(e) => eprintln!("Error listing tools: {e}"),
            }
        } else {
            eprintln!("No MCP client initialized");
        }
        eprintln!("======================\n");
    }

    // Initialize prompt override directory if needed
    let _ = crate::prompt_loader::init_prompt_override_dir();

    // Check for agent persona first - enables progressive tool loading
    let agent_purpose = extract_agent_role();
    let has_agent_persona = agent_purpose.as_ref().is_some_and(|p| !p.is_empty());

    // Build base system prompt (without repo context initially)
    // Use model-specific prompts based on model family and size
    let base_system_prompt = if has_agent_persona {
        // Agent mode: persona-focused, tools loaded progressively on demand
        let persona = agent_purpose.as_ref().unwrap();
        format!(
            "You are an AI agent with the following purpose:\n{persona}\n\nStay in character and focus on this purpose. Answer questions directly without using tools unless explicitly needed."
        )
    } else if mcp_client.is_some() && model_size_hint != Some("270M") {
        // Extract tool names for model-specific prompt
        let tool_names: Vec<String> = if let Some(ref client) = mcp_client {
            client
                .list_tools()
                .map(|tools| tools.iter().map(|t| t.name.clone()).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let tool_refs: Vec<&str> = tool_names.iter().map(|s| s.as_str()).collect();

        // Use model-specific prompt based on provider name
        let model_family = crate::prompt_loader::ModelFamily::from_name(&provider_name);
        if std::env::var("ARKAVO_DEBUG_CHAT").is_ok() {
            eprintln!("[DEBUG] Provider: {provider_name}, Family: {model_family:?}");
        }
        let model_prompt = crate::prompt_loader::load_model_chat_prompt(
            &provider_name,
            model_size_hint,
            if tool_refs.is_empty() {
                None
            } else {
                Some(&tool_refs)
            },
        );

        // For GLM models: use ONLY the model-specific prompt (no tool info)
        // GLM's MoE coding expert is triggered by tool mentions and causes crashes/loops
        if matches!(model_family, crate::prompt_loader::ModelFamily::Glm) {
            model_prompt
        } else if model_prompt.is_empty() {
            mcp_info
        } else {
            format!(
                "{model_prompt}\n\n\
                 Tool Discovery: If you need a capability not listed, annotate with [need: keyword].\n\
                 {mcp_info}"
            )
        }
    } else {
        // For 270M models or when MCP not available, use no system prompt
        String::new()
    };

    // Debug: Show system prompt being sent
    if std::env::var("ARKAVO_DEBUG_CHAT").is_ok() && !base_system_prompt.is_empty() {
        eprintln!("\n=== SYSTEM PROMPT ===");
        eprintln!("{base_system_prompt}");
        eprintln!("=====================\n");
    }

    // Determine whether to inject repo context
    let should_inject_context = if let (true, Some(ref p)) = (print_mode, prompt.as_ref()) {
        // For print mode with prompt, check if we should attach context
        let current_mode = REPO_CONTEXT_MODE.read().unwrap_or_else(|e| e.into_inner());
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
        let current_mode = REPO_CONTEXT_MODE.read().unwrap_or_else(|e| e.into_inner());
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
        // Try to read .arkavo/AGENTS.md first, then AGENTS.md, then CLAUDE.md
        let agents_md_content = if std::path::Path::new(".arkavo/AGENTS.md").exists() {
            std::fs::read_to_string(".arkavo/AGENTS.md").ok()
        } else {
            match std::fs::read_to_string("AGENTS.md") {
                Ok(content) => Some(content),
                Err(_) => std::fs::read_to_string("CLAUDE.md").ok(),
            }
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

    // Get conversation context with system message (only if non-empty)
    let system_message = if !system_prompt.is_empty() {
        Some(Message::system(&system_prompt))
    } else {
        None
    };

    let mut messages = if print_mode {
        // In print mode, just create a simple message list
        if let Some(msg) = system_message.clone() {
            vec![msg]
        } else {
            vec![]
        }
    } else {
        // In interactive mode, get context with appropriate history limits
        let history_limit = if model_size_hint == Some("270M") {
            Some(0) // No history for tiny models - fresh context each time
        } else if let Some("1B" | "2B") = model_size_hint {
            Some(4)
        } else {
            None // Use default
        };

        runtime.block_on(
            conversation_manager
                .get_context_messages_with_limits(system_message.clone(), history_limit),
        )?
    };

    // If prompt provided via command line, process it and exit
    if let Some(prompt_text) = prompt {
        // Check for pattern-based prompt adjustment
        let pattern_adjustment = crate::feedback_analyzer::check_pattern_adjustment_sync(
            client.provider_name(),
            &prompt_text,
        );

        let adjusted_prompt = if let Some(ref adj) = pattern_adjustment {
            if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
                eprintln!(
                    "[DEBUG] Applying pattern adjustment: prefix={:?}, max_tokens={:?}",
                    adj.prompt_prefix, adj.max_tokens
                );
            }
            if adj.prompt_prefix.is_empty() {
                prompt_text
            } else {
                format!("{}\n\n{prompt_text}", adj.prompt_prefix)
            }
        } else {
            prompt_text
        };

        // Apply max_tokens override if pattern adjustment suggests it (future use)
        let _effective_max_tokens = pattern_adjustment
            .as_ref()
            .and_then(|adj| adj.max_tokens)
            .map(|t| t as usize);

        // Check if image is also provided
        if let Some(img_path) = image_path {
            match encode_image_file(&img_path) {
                Ok(encoded_image) => {
                    messages.push(Message::user_with_images(
                        &adjusted_prompt,
                        vec![encoded_image],
                    ));
                }
                Err(e) => {
                    eprintln!("Error loading image: {e}");
                    messages.push(Message::user(&adjusted_prompt));
                }
            }
        } else {
            messages.push(Message::user(&adjusted_prompt));
        }

        // Disable MCP tools for persona-focused agents (progressive tool loading)
        let effective_mcp = if has_agent_persona {
            None
        } else {
            mcp_client.as_ref()
        };

        if print_mode {
            #[cfg(all(unix, feature = "mcp-tools"))]
            {
                runtime.block_on(process_message_print_with_router(&messages, effective_mcp))?;
            }
            #[cfg(not(all(unix, feature = "mcp-tools")))]
            {
                eprintln!("⚠️  WARNING: MCP tools not available on this platform");
                runtime.block_on(process_message_print(&client, &messages, effective_mcp))?;
            }
        } else {
            // Use router-based tool calling on supported platforms
            #[cfg(all(unix, feature = "mcp-tools"))]
            {
                runtime.block_on(process_message_with_tools(&messages, effective_mcp))?;
            }
            #[cfg(not(all(unix, feature = "mcp-tools")))]
            {
                runtime.block_on(process_message(
                    &client,
                    &messages,
                    effective_mcp,
                    &conversation_manager,
                ))?;
            }
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
            messages = if let Some(msg) = system_message.clone() {
                vec![msg]
            } else {
                vec![]
            };
            println!("Started new conversation session");
            continue;
        }

        // Handle /clear command - clear current session
        if input == "/clear" {
            conversation_manager.clear_session()?;
            let _ = runtime.block_on(conversation_manager.start_session(client.provider_name()))?;
            messages = if let Some(msg) = system_message.clone() {
                vec![msg]
            } else {
                vec![]
            };
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
                        let mut mode = REPO_CONTEXT_MODE.write().unwrap_or_else(|e| e.into_inner());
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
                    let mode = REPO_CONTEXT_MODE.read().unwrap_or_else(|e| e.into_inner());
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
                                conversation_manager.get_context_messages(system_message.clone()),
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
            // Add regular user message with pattern-based adjustment if available
            let adjusted_input = if let Some(adj) =
                crate::feedback_analyzer::check_pattern_adjustment_sync(
                    client.provider_name(),
                    input,
                ) {
                if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
                    eprintln!(
                        "[DEBUG] Applying pattern adjustment: prefix={:?}, max_tokens={:?}",
                        adj.prompt_prefix, adj.max_tokens
                    );
                }
                if adj.prompt_prefix.is_empty() {
                    input.to_string()
                } else {
                    format!("{}\n\n{input}", adj.prompt_prefix)
                }
            } else {
                input.to_string()
            };
            let msg = Message::user(&adjusted_input);
            runtime.block_on(conversation_manager.add_message(&msg))?;
            messages.push(msg);
        }

        // Process with LLM using router-based tool calling
        let result = {
            #[cfg(all(unix, feature = "mcp-tools"))]
            {
                runtime.block_on(process_message_with_tools(&messages, mcp_client.as_ref()))
            }
            #[cfg(not(all(unix, feature = "mcp-tools")))]
            {
                runtime.block_on(process_message(
                    &client,
                    &messages,
                    mcp_client.as_ref(),
                    &conversation_manager,
                ))
            }
        };

        match result {
            Ok(response) => {
                let assistant_msg = Message::assistant(&response);
                runtime.block_on(conversation_manager.add_message(&assistant_msg))?;
                messages.push(assistant_msg);

                // Check context size and offload if threshold exceeded
                let total_context_bytes: usize = messages.iter().map(|m| m.content.len()).sum();
                if total_context_bytes > context_offload_threshold
                    && messages.len() > preserve_recent_messages
                {
                    // Offload oldest messages (excluding system and recent messages)
                    let offload_count = messages.len().saturating_sub(preserve_recent_messages);
                    let has_system = messages
                        .first()
                        .is_some_and(|m| matches!(m.role, arkavo_llm::Role::System));
                    let start_idx = usize::from(has_system);
                    let end_idx = start_idx + offload_count.min(messages.len() - start_idx);

                    if end_idx > start_idx {
                        // Collect content to offload
                        let offload_content: String = messages[start_idx..end_idx]
                            .iter()
                            .map(|m| {
                                let role_str = match m.role {
                                    arkavo_llm::Role::User => "User",
                                    arkavo_llm::Role::Assistant => "Assistant",
                                    arkavo_llm::Role::System => "System",
                                };
                                format!("[{role_str}]: {}", m.content)
                            })
                            .collect::<Vec<_>>()
                            .join("\n\n");

                        // Generate summary (simple truncation if summarizer not available)
                        let summary = if offload_content.len() > 100 {
                            format!("{}...", &offload_content[..100])
                        } else {
                            offload_content.clone()
                        };

                        // Offload to ledger
                        match runtime.block_on(context_ledger.offload(
                            &offload_content,
                            &summary,
                            "chat-context",
                        )) {
                            Ok(pointer) => {
                                // Replace offloaded messages with a single pointer message
                                messages.drain(start_idx..end_idx);
                                let pointer_msg = Message::system(&pointer);
                                messages.insert(start_idx, pointer_msg);

                                if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
                                    eprintln!(
                                        "[DEBUG] Context offloaded: {} messages -> {}",
                                        end_idx - start_idx,
                                        pointer
                                    );
                                }
                            }
                            Err(e) => {
                                if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
                                    eprintln!("[DEBUG] Context offload failed: {e}");
                                }
                            }
                        }
                    }
                }

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

/// Process message with native tool calling via Router
#[cfg(all(unix, feature = "mcp-tools"))]
async fn process_message_with_tools(
    messages: &[Message],
    mcp_client: Option<&McpConnection>,
) -> Result<String, Box<dyn std::error::Error>> {
    use crate::tool_integration::process_with_tools_interactive;
    use std::sync::Arc;

    // Derive task description from the last user message
    let task_description = messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, arkavo_llm::Role::User))
        .map(|m| m.content.as_str())
        .unwrap_or("Process user request");

    print!("Assistant: ");
    io::stdout().flush()?;

    let progress = ProgressBar::new_spinner();
    progress.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    progress.set_message("Routing to LLM...");

    // Convert McpConnection to Arc<dyn McpClient> if present
    let mcp_arc = mcp_client.map(|c| Arc::new(c.clone()) as Arc<dyn arkavo_mcp_tools::McpClient>);

    let response =
        process_with_tools_interactive(task_description, messages.to_vec(), mcp_arc).await?;

    progress.finish_and_clear();
    println!("{response}");
    println!();

    Ok(response)
}

#[cfg(not(all(unix, feature = "mcp-tools")))]
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

#[cfg(all(unix, feature = "mcp-tools"))]
async fn process_message_print_with_router(
    messages: &[Message],
    mcp_client: Option<&McpConnection>,
) -> Result<String, Box<dyn std::error::Error>> {
    use crate::tool_integration::{ToolIntegrationConfig, process_with_tools};
    use std::sync::Arc;

    let task_description = messages
        .last()
        .map(|m| m.content.as_str())
        .unwrap_or("User query");

    let mcp_arc = mcp_client.map(|c| Arc::new(c.clone()) as Arc<dyn arkavo_mcp_tools::McpClient>);

    let config = ToolIntegrationConfig {
        max_tool_iterations: 10,
        show_tool_execution: false,
    };

    let result =
        process_with_tools(task_description, messages.to_vec(), Some(config), mcp_arc).await?;

    // Strip <think> blocks and print response cleanly
    let response = strip_think_blocks(&result.final_response);

    // Check if this was a math query - if so, extract just the answer to prevent loops
    let user_prompt_text = messages.last().map(|m| m.content.as_str()).unwrap_or("");
    let is_math = crate::feedback_analyzer::is_simple_math_query(user_prompt_text);

    // For math queries or if response is very long (likely looping), extract first answer
    let trimmed = if is_math || response.len() > 500 {
        crate::feedback_analyzer::extract_first_answer(&response, is_math)
    } else {
        response.trim().to_string()
    };

    // Debug: show what we're working with
    if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
        let resp_len = result.final_response.len();
        let strip_len = response.len();
        let trim_len = trimmed.len();
        let tool_count = result.tool_executions.len();
        eprintln!("[DEBUG] final_response length: {resp_len}");
        eprintln!("[DEBUG] stripped length: {strip_len}");
        eprintln!("[DEBUG] trimmed length: {trim_len}");
        eprintln!("[DEBUG] is_math: {is_math}");
        eprintln!("[DEBUG] tool_executions count: {tool_count}");
    }

    // If response is empty but we executed tools, show tool results directly
    if trimmed.is_empty() && !result.tool_executions.is_empty() {
        // Extract display text from tool results (tools provide their own summaries)
        for exec in &result.tool_executions {
            if exec.success {
                // Check for display/summary field first, fall back to result JSON
                if let Some(display) = exec.result.get("display").and_then(|v| v.as_str()) {
                    println!("{display}");
                } else if let Ok(result_str) = serde_json::to_string_pretty(&exec.result) {
                    println!("{result_str}");
                }
            }
        }
    } else if !trimmed.is_empty() {
        println!("{trimmed}");
    }

    if !result.tool_executions.is_empty() {
        eprintln!(
            "\n✓ Executed {} tool(s) across {} iteration(s)",
            result.tool_executions.len(),
            result.total_iterations
        );
    }

    // Record feedback for learning (analyze response for issues)
    let user_prompt = messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, arkavo_llm::Role::User))
        .map(|m| m.content.clone())
        .unwrap_or_default();

    // Get model name from router - use a generic name since router handles model selection
    let model_name = "router-glm"; // Router typically uses GLM for local inference
    if let Err(e) = crate::feedback_analyzer::record_model_feedback(
        model_name,
        &user_prompt,
        &result.final_response,
    )
    .await
        && SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed)
    {
        eprintln!("[DEBUG] Failed to record feedback: {e}");
    }

    Ok(result.final_response)
}

#[cfg(not(all(unix, feature = "mcp-tools")))]
async fn process_message_print(
    client: &LlmClient,
    messages: &[Message],
    mcp_client: Option<&McpConnection>,
) -> Result<String, Box<dyn std::error::Error>> {
    use std::time::Instant;

    let start_time = Instant::now();

    // Extract the last user prompt for feedback recording
    let user_prompt = messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, arkavo_llm::Role::User))
        .map(|m| m.content.clone())
        .unwrap_or_default();
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

    // Check if RLM mode would be beneficial for this context size
    let full_context: String = messages.iter().map(|m| m.content.as_str()).collect();
    // Derive model info from provider name
    let provider_name = client.provider_name().to_lowercase();
    let model_size_hint = if provider_name.contains("270m") {
        Some("270M")
    } else if provider_name.contains("1b") {
        Some("1B")
    } else if provider_name.contains("7b") {
        Some("7B")
    } else {
        None
    };
    let is_large_context_provider = provider_name.contains("gemini")
        || provider_name.contains("claude")
        || provider_name.contains("gpt")
        || provider_name.contains("anthropic");

    if let Some(rlm_hint) =
        check_rlm_activation(&full_context, model_size_hint, is_large_context_provider)
    {
        eprintln!("[RLM] {}", rlm_hint);
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

                        // Record timeout feedback for learning
                        if let Err(e) = crate::feedback_analyzer::record_timeout_feedback(
                            client.provider_name(),
                            &user_prompt,
                        )
                        .await
                        {
                            if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
                                eprintln!("[DEBUG] Failed to record timeout feedback: {e}");
                            }
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

            // Record feedback for learning (analyze response for issues)
            if let Err(e) = crate::feedback_analyzer::record_model_feedback(
                client.provider_name(),
                &user_prompt,
                &full_response,
            )
            .await
            {
                if SHOW_DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
                    eprintln!("[DEBUG] Failed to record feedback: {e}");
                }
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

fn get_current_directory() -> String {
    match env::current_dir() {
        Ok(path) => path.display().to_string(),
        Err(_) => String::from("Unknown"),
    }
}

#[cfg(all(unix, feature = "mcp-tools"))]
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

// Type alias for tool execution results (used by handle_tool_calls_in_response in feature-gated code)
#[allow(dead_code)]
type ToolResults = Vec<(String, String)>;

// Stub implementations when mcp-tools is not available
#[cfg(not(all(unix, feature = "mcp-tools")))]
fn handle_command(
    _input: &str,
    _mcp_client: Option<&McpConnection>,
    _llm_provider: &str,
) -> Option<String> {
    None
}

#[cfg(not(all(unix, feature = "mcp-tools")))]
fn handle_tool_calls_in_response(
    response: &str,
    _mcp_client: &McpConnection,
    _llm_provider: &str,
) -> Result<(String, ToolResults), Box<dyn std::error::Error>> {
    // When MCP is not available, just return the response as-is
    Ok((response.to_string(), Vec::new()))
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
            Some(format!("Content of {file_path}:\n\n{preview}"))
        }
        Err(e) => Some(format!("Error reading file '{file_path}': {e}")),
    }
}

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

pub async fn initialize_llm_for_ui(
    model_name: &str,
) -> Result<LlmClient, Box<dyn std::error::Error>> {
    // Simplified version for UI command - uses default params
    initialize_llm_client(false, model_name, 0.7, 0.9, 40, 4096, 42).await
}

/// List local GGUF models from HuggingFace cache
fn list_local_gguf_models() -> Vec<(String, String, std::path::PathBuf, u64)> {
    let mut found_models = Vec::new();

    let hf_cache_dir = if let Ok(hf_home) = std::env::var("HF_HOME") {
        Some(std::path::PathBuf::from(hf_home).join("hub"))
    } else {
        dirs::home_dir().map(|d| d.join(".cache").join("huggingface").join("hub"))
    };

    if let Some(cache_dir) = hf_cache_dir
        && cache_dir.exists()
        && let Ok(entries) = std::fs::read_dir(&cache_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if dir_name.starts_with("models--") {
                    let snapshots_dir = path.join("snapshots");
                    if snapshots_dir.exists()
                        && let Ok(snapshot_entries) = std::fs::read_dir(&snapshots_dir)
                    {
                        for snapshot in snapshot_entries.flatten() {
                            let snapshot_path = snapshot.path();
                            if snapshot_path.is_dir()
                                && let Ok(files) = std::fs::read_dir(&snapshot_path)
                            {
                                for file in files.flatten() {
                                    if let Some(name) = file.file_name().to_str()
                                        && name.ends_with(".gguf")
                                    {
                                        let model_name = dir_name
                                            .strip_prefix("models--")
                                            .unwrap_or(dir_name)
                                            .replace("--", "/");
                                        let file_path = file.path();
                                        let size = std::fs::metadata(&file_path)
                                            .map(|m| m.len())
                                            .unwrap_or(0);
                                        found_models.push((
                                            model_name,
                                            name.to_string(),
                                            file_path,
                                            size,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    found_models
}

/// Find a compatible model in the HuggingFace cache
fn find_compatible_cached_model() -> Option<(String, String, std::path::PathBuf)> {
    let models = list_local_gguf_models();

    // Priority order: Qwen3, Ministral, Gemma
    for (repo, file, path, _size) in &models {
        let name_lower = repo.to_lowercase();
        if name_lower.contains("qwen") {
            return Some((repo.clone(), file.clone(), path.clone()));
        }
    }
    for (repo, file, path, _size) in &models {
        let name_lower = repo.to_lowercase();
        if name_lower.contains("ministral") || name_lower.contains("mistral") {
            return Some((repo.clone(), file.clone(), path.clone()));
        }
    }
    for (repo, file, path, _size) in &models {
        let name_lower = repo.to_lowercase();
        if name_lower.contains("gemma") {
            return Some((repo.clone(), file.clone(), path.clone()));
        }
    }

    None
}

/// Initialize LLM with a compatible cached model or prompt to download Qwen3 + Ministral
#[cfg(feature = "llama-cpp")]
async fn initialize_with_compatible_model_or_download(
    print_mode: bool,
    temperature: f32,
    top_p: f32,
    top_k: i32,
    max_tokens: u32,
    seed: u32,
) -> Result<LlmClient, Box<dyn std::error::Error>> {
    use hf_hub::api::tokio::Api;

    // Check for compatible models in cache first
    if let Some((repo, file, path)) = find_compatible_cached_model() {
        if !print_mode {
            println!("Found compatible model: {repo}/{file}");
        }
        return LlmClient::from_llamacpp_model_with_config(
            file.trim_end_matches(".gguf"),
            path.to_string_lossy().to_string(),
            temperature,
            top_p,
            top_k,
            max_tokens,
            seed,
        )
        .await
        .map_err(|e| e.into());
    }

    // No compatible model found - prompt user to download
    if !print_mode {
        println!("\nNo compatible models found. Arkavo Edge uses multiple models via the router.");
        println!("The following models will be downloaded:");
        println!("  • Qwen3-0.6B (~600MB) - Fast, for simple tasks");
        println!("  • Ministral-3B (~2GB) - Higher quality reasoning");
        print!("\nDownload both models now? (Y/n) ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if input.trim().to_lowercase() == "n" {
            return Err("Download declined by user.".into());
        }
    }

    let api = Api::new()?;

    // Download Qwen3-0.6B
    if !print_mode {
        println!("\nDownloading Qwen3-0.6B...");
    }
    let qwen_repo = api.repo(hf_hub::Repo::model("Qwen/Qwen3-0.6B-GGUF".to_string()));
    let qwen_path = qwen_repo.get("Qwen3-0.6B-Q8_0.gguf").await?;
    if !print_mode {
        println!("  ✓ Qwen3-0.6B downloaded");
    }

    // Download Ministral-3B
    if !print_mode {
        println!("Downloading Ministral-3B...");
    }
    let ministral_repo = api.repo(hf_hub::Repo::model(
        "mistralai/Ministral-3-3B-Instruct-2512-GGUF".to_string(),
    ));
    let _ministral_path = ministral_repo
        .get("Ministral-3-3B-Instruct-2512-Q4_K_M.gguf")
        .await?;
    if !print_mode {
        println!("  ✓ Ministral-3B downloaded");
        println!("\nModels ready. Using Qwen3-0.6B for this session.");
    }

    LlmClient::from_llamacpp_model_with_config(
        "qwen3-0.6b",
        qwen_path.to_string_lossy().to_string(),
        temperature,
        top_p,
        top_k,
        max_tokens,
        seed,
    )
    .await
    .map_err(|e| e.into())
}

async fn initialize_llm_client(
    print_mode: bool,
    model_name: &str,
    #[cfg_attr(not(feature = "llama-cpp"), allow(unused_variables))] temperature: f32,
    #[cfg_attr(not(feature = "llama-cpp"), allow(unused_variables))] top_p: f32,
    #[cfg_attr(not(feature = "llama-cpp"), allow(unused_variables))] top_k: i32,
    #[cfg_attr(not(feature = "llama-cpp"), allow(unused_variables))] max_tokens: u32,
    #[cfg_attr(not(feature = "llama-cpp"), allow(unused_variables))] seed: u32,
) -> Result<LlmClient, Box<dyn std::error::Error>> {
    // Check if model_name specifies a provider directly
    if model_name == "deepseek" {
        // Create config for DeepSeek provider (reads API key from env at startup)
        let config = LlmConfig::from_env();
        let config = LlmConfig {
            provider: "deepseek".to_string(),
            ..config
        };
        if let Ok(client) = LlmClient::from_config(&config) {
            if !print_mode {
                println!("✓ Connected to DeepSeek API");
            }
            return Ok(client);
        } else if !print_mode {
            println!("Failed to connect to DeepSeek. Check DEEPSEEK_API_KEY environment variable.");
        }
    } else if model_name == "kimi" {
        // Create config for Kimi provider
        let config = LlmConfig::from_env();
        let config = LlmConfig {
            provider: "kimi".to_string(),
            ..config
        };
        if let Ok(client) = LlmClient::from_config(&config) {
            if !print_mode {
                println!("✓ Connected to Kimi API");
            }
            return Ok(client);
        } else if !print_mode {
            println!("Failed to connect to Kimi. Check MOONSHOT_API_KEY environment variable.");
        }
    } else if model_name == "gemini" || model_name.starts_with("gemini-") {
        // Create config for Gemini provider
        let mut config = LlmConfig::from_env();
        config.provider = "gemini".to_string();
        if model_name.starts_with("gemini-") {
            config.model = Some(model_name.to_string());
        }
        if let Ok(client) = LlmClient::from_config(&config) {
            if !print_mode {
                println!("✓ Connected to Gemini API");
            }
            return Ok(client);
        } else if !print_mode {
            println!("Failed to connect to Gemini. Check GEMINI_API_KEY environment variable.");
        }
    }

    // Try to connect to Ollama for other models
    let config = LlmConfig::ollama();
    if let Ok(client) = LlmClient::from_config(&config)
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

        // Check if model_name is a path to a GGUF file or a model identifier
        let final_path = if model_name.ends_with(".gguf")
            || model_name.contains('/') && std::path::Path::new(model_name).exists()
        {
            // Direct path to GGUF file
            if !print_mode {
                println!("Loading model from path: {model_name}");
            }
            std::path::PathBuf::from(model_name)
        } else {
            // Model identifier - try to find it in HF cache or download
            let (model_repo, model_file, display_name) = if model_name.contains('/') {
                // Format: repo/file or repo/model
                if model_name.contains(".gguf") {
                    // Full repo/file.gguf format
                    let parts: Vec<&str> = model_name.rsplitn(2, '/').collect();
                    let file = parts[0];
                    let repo = parts[1];
                    (repo, file, file.trim_end_matches(".gguf"))
                } else {
                    // Just repo name, need to guess the file
                    match model_name {
                        "bartowski/gemma-2-2b-it-GGUF" | "gemma-2-2b" | "gemma-2b" => (
                            "bartowski/gemma-2-2b-it-GGUF",
                            "gemma-2-2b-it-Q4_K_M.gguf",
                            "gemma-2-2b-it",
                        ),
                        "bartowski/gemma-2-9b-it-GGUF" | "gemma-2-9b" | "gemma-9b" | "gemma9" => (
                            "bartowski/gemma-2-9b-it-GGUF",
                            "gemma-2-9b-it-Q4_K_M.gguf",
                            "gemma-2-9b-it",
                        ),
                        "unsloth/gemma-3-4b-it-GGUF" | "gemma-3-4b" | "gemma-4b" | "gemma4" => (
                            "unsloth/gemma-3-4b-it-GGUF",
                            "gemma-3-4b-it-Q4_K_M.gguf",
                            "gemma-3-4b-it",
                        ),
                        "TheBloke/TinyLlama-1.1B-Chat-v1.0-GGUF" | "tinyllama" => (
                            "TheBloke/TinyLlama-1.1B-Chat-v1.0-GGUF",
                            "tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf",
                            "tinyllama-1.1b",
                        ),
                        "TheBloke/phi-2-GGUF" | "phi-2" => {
                            ("TheBloke/phi-2-GGUF", "phi-2.Q4_K_M.gguf", "phi-2")
                        }
                        // Ministral 3 models (official Mistral repos)
                        "mistralai/Ministral-3-3B-Instruct-2512-GGUF"
                        | "ministral-3b"
                        | "ministral3b" => (
                            "mistralai/Ministral-3-3B-Instruct-2512-GGUF",
                            "Ministral-3-3B-Instruct-2512-Q4_K_M.gguf",
                            "ministral-3-3b",
                        ),
                        "mistralai/Ministral-3-8B-Instruct-2512-GGUF"
                        | "ministral-8b"
                        | "ministral8b" => (
                            "mistralai/Ministral-3-8B-Instruct-2512-GGUF",
                            "Ministral-3-8B-Instruct-2512-Q4_K_M.gguf",
                            "ministral-3-8b",
                        ),
                        "mistralai/Ministral-3-14B-Instruct-2512-GGUF"
                        | "ministral-14b"
                        | "ministral14b" => (
                            "mistralai/Ministral-3-14B-Instruct-2512-GGUF",
                            "Ministral-3-14B-Instruct-2512-Q4_K_M.gguf",
                            "ministral-3-14b",
                        ),
                        // Qwen3 models
                        "Qwen/Qwen3-0.6B-GGUF" | "qwen3-0.6b" | "qwen-0.6b" | "qwen3" | "qwen" => {
                            ("Qwen/Qwen3-0.6B-GGUF", "Qwen3-0.6B-Q8_0.gguf", "qwen3-0.6b")
                        }
                        // GLM-4.7-Flash (30B MoE reasoning model)
                        "unsloth/GLM-4.7-Flash-GGUF" | "glm-4.7" | "glm" => (
                            "unsloth/GLM-4.7-Flash-GGUF",
                            "GLM-4.7-Flash-Q4_K_M.gguf",
                            "glm-4.7-flash",
                        ),
                        _ => {
                            // Check cache for compatible models or prompt to download
                            return initialize_with_compatible_model_or_download(
                                print_mode,
                                temperature,
                                top_p,
                                top_k,
                                max_tokens,
                                seed,
                            )
                            .await;
                        }
                    }
                }
            } else {
                // Simple model name
                match model_name {
                    "gemma-2-2b" | "gemma-2b" | "gemma2" => (
                        "bartowski/gemma-2-2b-it-GGUF",
                        "gemma-2-2b-it-Q4_K_M.gguf",
                        "gemma-2-2b-it",
                    ),
                    "gemma-2-9b" | "gemma-9b" | "gemma9" => (
                        "bartowski/gemma-2-9b-it-GGUF",
                        "gemma-2-9b-it-Q4_K_M.gguf",
                        "gemma-2-9b-it",
                    ),
                    "gemma-3-4b" | "gemma-4b" | "gemma4" => (
                        "unsloth/gemma-3-4b-it-GGUF",
                        "gemma-3-4b-it-Q4_K_M.gguf",
                        "gemma-3-4b-it",
                    ),
                    "tinyllama" | "tiny" => (
                        "TheBloke/TinyLlama-1.1B-Chat-v1.0-GGUF",
                        "tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf",
                        "tinyllama-1.1b",
                    ),
                    "phi-2" | "phi2" => ("TheBloke/phi-2-GGUF", "phi-2.Q4_K_M.gguf", "phi-2"),
                    // Ministral 3 models (official Mistral repos)
                    "ministral-3b" | "ministral3b" | "ministral-3-3b" => (
                        "mistralai/Ministral-3-3B-Instruct-2512-GGUF",
                        "Ministral-3-3B-Instruct-2512-Q4_K_M.gguf",
                        "ministral-3-3b",
                    ),
                    "ministral-8b" | "ministral8b" | "ministral-3-8b" => (
                        "mistralai/Ministral-3-8B-Instruct-2512-GGUF",
                        "Ministral-3-8B-Instruct-2512-Q4_K_M.gguf",
                        "ministral-3-8b",
                    ),
                    "ministral-14b" | "ministral14b" | "ministral-3-14b" => (
                        "mistralai/Ministral-3-14B-Instruct-2512-GGUF",
                        "Ministral-3-14B-Instruct-2512-Q4_K_M.gguf",
                        "ministral-3-14b",
                    ),
                    // Qwen3 models
                    "qwen3-0.6b" | "qwen-0.6b" | "qwen3" | "qwen" => {
                        ("Qwen/Qwen3-0.6B-GGUF", "Qwen3-0.6B-Q8_0.gguf", "qwen3-0.6b")
                    }
                    // GLM-4.7-Flash (30B MoE reasoning model)
                    "glm-4.7" | "glm" => (
                        "unsloth/GLM-4.7-Flash-GGUF",
                        "GLM-4.7-Flash-Q4_K_M.gguf",
                        "glm-4.7-flash",
                    ),
                    _ => {
                        // Check cache for compatible models or prompt to download
                        return initialize_with_compatible_model_or_download(
                            print_mode,
                            temperature,
                            top_p,
                            top_k,
                            max_tokens,
                            seed,
                        )
                        .await;
                    }
                }
            };

            if !print_mode {
                println!("Loading model: {display_name}");
            }

            let api = Api::new()?;
            let repo = api.repo(hf_hub::Repo::model(model_repo.to_string()));
            let model_path = repo.get(model_file).await;

            match model_path {
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
            }
        };

        // Extract display name from path if needed
        let display_name = if model_name.ends_with(".gguf")
            || model_name.contains('/') && std::path::Path::new(model_name).exists()
        {
            final_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("model")
        } else {
            model_name
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
