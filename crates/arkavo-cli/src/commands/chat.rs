use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::runtime::Runtime;

// Re-export types from arkavo-protocol for use by other commands and tests
pub use arkavo_protocol::{
    ChatSession, CommandResult, ContextMode, PendingContext, execute_command,
    parse_command as parse_protocol_command,
};

// Re-export types from migrated modules for use by other commands
pub use arkavo_context::{RepoContextMode, compress_repo_context, should_attach_repo_context};
pub use arkavo_router::response::{sanitize_response, strip_think_blocks, strip_tool_blocks};
pub use arkavo_session::{ConversationManager, ConversationMessage, ConversationSession};

// Global flag to control whether to show debug messages
// Set via --debug command line flag
pub(crate) static SHOW_DEBUG: AtomicBool = AtomicBool::new(false);

/// Interactive chat commands (kept for backwards compatibility)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatCommand {
    /// Start a new conversation session
    New,
    /// Clear the current conversation context
    Clear,
    /// Set repository context mode (off, on, auto)
    Context(String),
    /// Show conversation history
    History,
    /// Switch to a different conversation session
    Switch(String),
    /// Read a file into context
    Read(String),
    /// List files in a directory
    List(Option<String>),
    /// Exit the chat
    Exit,
    /// Show help
    Help,
}

/// Parse a command from user input
///
/// Returns None if the input is not a command (doesn't start with /)
pub fn parse_command(input: &str) -> Option<ChatCommand> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }

    let parts: Vec<&str> = trimmed[1..].splitn(2, ' ').collect();
    let cmd = parts[0].to_lowercase();
    let arg = parts.get(1).map(|s| s.trim().to_string());

    match cmd.as_str() {
        "new" => Some(ChatCommand::New),
        "clear" => Some(ChatCommand::Clear),
        "context" => Some(ChatCommand::Context(arg.unwrap_or_default())),
        "history" => Some(ChatCommand::History),
        "switch" => arg.map(ChatCommand::Switch),
        "read" => arg.map(ChatCommand::Read),
        "list" | "ls" => Some(ChatCommand::List(arg)),
        "exit" | "quit" | "q" => Some(ChatCommand::Exit),
        "help" | "?" => Some(ChatCommand::Help),
        _ => None,
    }
}

fn create_runtime() -> std::io::Result<Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_name("arkavo-chat-worker")
        .thread_stack_size(3 * 1024 * 1024)
        .enable_all()
        .build()
}

/// Execute the chat command
#[allow(clippy::disallowed_methods)]
pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // Check for --help flag first
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_usage();
        return Ok(());
    }

    // Check for --debug flag in arguments
    if args.iter().any(|arg| arg == "--debug") {
        SHOW_DEBUG.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    // Parse --prompt, --agent-id, and --model from args
    let mut prompt: Option<String> = None;
    let mut agent_id: Option<String> = None;
    let mut model_name: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--prompt" | "--print" => {
                if i + 1 < args.len() {
                    prompt = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--agent-id" => {
                if i + 1 < args.len() {
                    agent_id = Some(args[i + 1].clone());
                    i += 1;
                } else {
                    return Err("--agent-id requires an argument".into());
                }
            }
            "--model" => {
                if i + 1 < args.len() {
                    model_name = Some(args[i + 1].clone());
                    i += 1;
                } else {
                    return Err(
                        "--model requires a model name (e.g., ministral-3b, qwen3.5-0.8b)".into(),
                    );
                }
            }
            _ => {}
        }
        i += 1;
    }

    // Direct A2A chat with a specific mesh agent
    if let Some(id) = agent_id {
        return execute_a2a_direct_chat(&id, prompt.as_deref());
    }

    // Default: route through local in-process Router
    execute_a2a_chat(prompt.as_deref(), model_name.as_deref())
}

fn print_usage() {
    println!("Start interactive chat with LLM\n");
    println!("USAGE:");
    println!("    arkavo chat                                   Interactive chat mode");
    println!("    arkavo chat --prompt \"query\"                  One-shot query");
    println!("    arkavo chat --agent-id <ID>                   Chat with a mesh agent");
    println!("    arkavo chat --agent-id <ID> --prompt \"query\"  One-shot to mesh agent\n");
    println!("EXAMPLES:");
    println!("    arkavo chat");
    println!("    arkavo chat --prompt \"What is 2+2?\"");
    println!("    arkavo chat --model ministral-3b --prompt \"What time is it?\"");
    println!("    arkavo chat --agent-id security-auditor-agent --prompt \"Audit this code\"");
    println!("    arkavo chat --agent-id code-analyzer-agent\n");
    println!("OPTIONS:");
    println!(
        "    --model <NAME>         Override model (e.g., ministral-3b, qwen3.5-0.8b, glm-4.7-flash)"
    );
    println!("    --agent-id <ID>        Chat directly with a mesh agent via A2A");
    println!("    --prompt <TEXT>         One-shot query (exits after response)");
    println!("    --debug                 Show debug output");
    println!("    -h, --help              Show this help\n");
    println!("INTERACTIVE COMMANDS:");
    println!("    /new              Start a new conversation");
    println!("    /clear            Clear conversation context");
    println!("    /context <mode>   Set repo context mode (off, on, auto)");
    println!("    /history          Show conversation history");
    println!("    /read <file>      Read a file into context");
    println!("    /list [dir]       List files in directory");
    println!("    /exit, /quit      Exit chat");
    println!("    /help             Show all commands");
}

/// Execute A2A chat mode using ChatSession from arkavo-protocol
///
/// Uses a local runtime (not `'static`) so that all Rust objects — including
/// the Router's ModelRegistry and its Metal-backed llama.cpp contexts — are
/// dropped deterministically before C++ static destructors run at process exit.
/// A `'static` runtime would keep `Arc<Router>` alive past `main()`, causing
/// `ggml_metal_device_free` to assert on non-empty residual sets.
#[allow(clippy::disallowed_methods)]
fn execute_a2a_chat(
    prompt: Option<&str>,
    model_name: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = create_runtime()?;

    runtime.block_on(async {
        // Initialize router
        let router = arkavo_router::Router::new().await?;

        // Initialize memory storage for tool registry
        let storage = Arc::new(arkavo_memory::storage::MemoryStorage::new().await?);

        // Create tool registry with built-in tools (time, filesystem, etc.)
        let tool_registry = arkavo_mcp_tools::ToolRegistry::new(storage);

        // Create ChatSession (wraps A2aClient)
        let mut session = ChatSession::new_with_model(
            Arc::new(router),
            Some(Arc::new(tool_registry)),
            model_name,
        )
        .await?;

        if std::env::var("ARKAVO_DEBUG").is_ok()
            && let Some(id) = session.session_id()
        {
            eprintln!("[A2A] Session: {}", &id[..8.min(id.len())]);
        }

        // One-shot mode
        if let Some(prompt) = prompt {
            let mut rx = session.send_message(prompt).await?;
            process_stream(&mut rx).await;
            session.cmd_exit().await;
            return Ok(());
        }

        // Interactive REPL
        println!("A2A Chat Mode (type /help for commands, /exit to quit)");
        loop {
            let input = read_user_input()?;

            // Check for commands
            if let Some((cmd, arg)) = parse_protocol_command(&input) {
                let result = execute_command(&mut session, cmd, arg).await;
                match result {
                    CommandResult::Success(msg) => {
                        if let Some(m) = msg {
                            println!("{m}");
                        }
                    }
                    CommandResult::Output(text) => {
                        println!("{text}");
                    }
                    CommandResult::Exit => break,
                    CommandResult::Error(err) => {
                        eprintln!("Error: {err}");
                    }
                }
                continue;
            }

            // Regular message - send to LLM
            if input.is_empty() {
                continue;
            }

            match session.send_message(&input).await {
                Ok(mut rx) => {
                    process_stream(&mut rx).await;
                }
                Err(e) => {
                    eprintln!("Error sending message: {e}");
                }
            }
        }

        Ok(())
    })
}

/// Chat directly with a mesh agent via A2A protocol (bypasses local Router)
#[allow(clippy::disallowed_methods)]
fn execute_a2a_direct_chat(
    agent_id: &str,
    prompt: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    use arkavo_protocol::{
        http::HttpTransport,
        transport::{A2aEndpoint, A2aTransport, TlsConfig, TransportConfig},
    };

    let discovered = super::mesh::discover_mesh_agents()?;
    if discovered.is_empty() {
        return Err("No mesh agents discovered. Are agents running?".into());
    }

    // Match by agent_id (exact) or name substring (fuzzy)
    let agent = discovered
        .iter()
        .find(|a| a.agent_id == agent_id)
        .or_else(|| {
            let lower = agent_id.to_lowercase();
            discovered
                .iter()
                .find(|a| a.name.to_lowercase().contains(&lower))
        })
        .ok_or_else(|| {
            let available: Vec<_> = discovered
                .iter()
                .map(|a| format!("  {} ({})", a.name, a.agent_id))
                .collect();
            format!(
                "Agent '{}' not found. Available agents:\n{}",
                agent_id,
                available.join("\n")
            )
        })?;

    let address = agent
        .address
        .as_ref()
        .ok_or("Selected agent has no address")?;

    println!("Connecting to {} ({})...", agent.name, agent.agent_id);
    println!("  Address: {address}");

    let runtime = create_runtime()?;
    runtime.block_on(async {
        let transport_config = TransportConfig {
            timeout_ms: 60000,
            max_retries: 2,
            tls_config: TlsConfig {
                require_tls: false,
                ..Default::default()
            },
            ..Default::default()
        };

        let transport = HttpTransport::new(transport_config)?;
        let endpoint = A2aEndpoint {
            url: address.clone(),
            agent_id: agent.agent_id.clone(),
            public_key: None,
        };

        transport
            .connect(&endpoint)
            .await
            .map_err(|e| format!("Failed to connect: {e}"))?;
        println!("  Connected\n");

        // One-shot mode
        if let Some(text) = prompt {
            super::mesh::send_and_poll_agent(&transport, text).await?;
            transport.close().await.ok();
            return Ok(());
        }

        // Interactive REPL
        println!("Direct chat with {} (type /exit to quit)", agent.name);
        loop {
            let input = read_user_input()?;
            if input.is_empty() {
                continue;
            }
            if input == "/exit" || input == "/quit" || input == "/q" {
                break;
            }
            match super::mesh::send_and_poll_agent(&transport, &input).await {
                Ok(()) => {}
                Err(e) => eprintln!("Error: {e}"),
            }
        }

        transport.close().await.ok();
        Ok(())
    })
}

/// Process streaming response
async fn process_stream(
    rx: &mut tokio::sync::mpsc::Receiver<arkavo_protocol::types::MessageDelta>,
) {
    use arkavo_protocol::types::MessageDeltaContent;

    let debug = std::env::var("ARKAVO_DEBUG").is_ok();
    let mut buf = String::new();
    let mut in_think = false;

    while let Some(delta) = rx.recv().await {
        match &delta.delta {
            MessageDeltaContent::Text { text } => {
                if debug {
                    // Debug mode: show everything including think blocks
                    print!("{text}");
                    let _ = io::stdout().flush();
                    continue;
                }

                buf.push_str(text);

                // Process buffer for think block boundaries
                loop {
                    if in_think {
                        if let Some(end) = buf.find("</think>") {
                            // Discard think content, resume after closing tag
                            buf = buf[end + "</think>".len()..].to_string();
                            in_think = false;
                        } else {
                            // Still inside think block — keep buffering
                            // Truncate to avoid unbounded growth, keep tail for partial tag match
                            // Tail must be >= len("</think>") - 1 = 7 to detect tags across chunks
                            if buf.len() > 1024 {
                                let mut tail_start = buf.len() - 16;
                                // Find nearest char boundary to avoid panic on multi-byte UTF-8
                                while !buf.is_char_boundary(tail_start) {
                                    tail_start += 1;
                                }
                                buf = buf[tail_start..].to_string();
                            }
                            break;
                        }
                    } else if let Some(start) = buf.find("<think>") {
                        // Print everything before the tag
                        let before = &buf[..start];
                        if !before.is_empty() {
                            print!("{before}");
                            let _ = io::stdout().flush();
                        }
                        buf = buf[start + "<think>".len()..].to_string();
                        in_think = true;
                    } else if let Some(end) = buf.find("</think>") {
                        // Fallback: model emitted <think> as a special token (now injected
                        // by the streaming layer), but in case it was missed, discard
                        // everything before the closing tag
                        buf = buf[end + "</think>".len()..].to_string();
                    } else {
                        // No tag found — flush safe portion, keep tail for partial tag match
                        // len("</think>") = 8, which is longer than "<think>" = 7
                        let safe_len = buf.len().saturating_sub(8);
                        if safe_len > 0 {
                            print!("{}", &buf[..safe_len]);
                            let _ = io::stdout().flush();
                            buf = buf[safe_len..].to_string();
                        }
                        break;
                    }
                }
            }
            MessageDeltaContent::ToolCall { name, .. } => {
                if let Some(name) = name
                    && debug
                {
                    eprintln!("\n[Tool: {name}]");
                }
            }
            MessageDeltaContent::ToolResult {
                content, is_error, ..
            } => {
                if *is_error {
                    eprintln!("[Error: {content}]");
                }
            }
            MessageDeltaContent::StreamEnd { .. } => {
                // Flush remaining buffer (only if not inside a think block)
                if !in_think && !buf.is_empty() {
                    print!("{buf}");
                }
                println!();
                break;
            }
            MessageDeltaContent::Error { message, .. } => {
                eprintln!("\n[Error: {message}]");
                break;
            }
            MessageDeltaContent::Metadata { key, value } => {
                if debug {
                    match key.as_str() {
                        "model_selected" => {
                            let model = value.get("model").and_then(|v| v.as_str()).unwrap_or("?");
                            let reason = value
                                .get("reasoning")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            eprintln!("[Model] {model} ({reason})");
                        }
                        "quality_feedback" => {
                            let latency = value
                                .get("latency_ms")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            let tool_count = value
                                .get("tool_call_count")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            let resp_len = value
                                .get("response_len")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            eprint!("[Perf] {latency}ms, {resp_len} chars");
                            if tool_count > 0 {
                                let names = value
                                    .get("tool_names")
                                    .and_then(|v| v.as_array())
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|v| v.as_str())
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    })
                                    .unwrap_or_default();
                                eprint!(", {tool_count} tool(s): [{names}]");
                            }
                            // Inference timing from local model
                            if let Some(gen_ms) =
                                value.get("generation_ms").and_then(|v| v.as_f64())
                            {
                                let prompt_ms = value
                                    .get("prompt_eval_ms")
                                    .and_then(|v| v.as_f64())
                                    .unwrap_or(0.0);
                                let prompt_tok = value
                                    .get("prompt_tokens")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let gen_tok = value
                                    .get("generated_tokens")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let tok_s = value
                                    .get("tokens_per_sec")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("?");
                                eprint!(
                                    " | eval: {prompt_ms:.0}ms/{prompt_tok}tok, gen: {gen_ms:.0}ms/{gen_tok}tok ({tok_s} tok/s)"
                                );
                            }
                            eprintln!();
                        }
                        "tool_search" => {
                            let keywords = value
                                .get("keywords")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?");
                            let found = value
                                .get("tools_found")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            let names = value
                                .get("tool_names")
                                .and_then(|v| v.as_array())
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|v| v.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                })
                                .unwrap_or_default();
                            eprintln!("[Tools] searched \"{keywords}\" → {found} found: [{names}]");
                        }
                        _ => {
                            eprintln!("[{key}] {value}");
                        }
                    }
                }
            }
        }
    }
}

/// Read user input for REPL
fn read_user_input() -> Result<String, Box<dyn std::error::Error>> {
    print!("> ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_new_command() {
        let cmd = parse_command("/new");
        assert!(matches!(cmd, Some(ChatCommand::New)));
    }

    #[test]
    fn test_parse_clear_command() {
        let cmd = parse_command("/clear");
        assert!(matches!(cmd, Some(ChatCommand::Clear)));
    }

    #[test]
    fn test_parse_context_command() {
        let cmd = parse_command("/context auto");
        assert!(matches!(cmd, Some(ChatCommand::Context(mode)) if mode == "auto"));

        let cmd = parse_command("/context on");
        assert!(matches!(cmd, Some(ChatCommand::Context(mode)) if mode == "on"));
    }

    #[test]
    fn test_parse_history_command() {
        let cmd = parse_command("/history");
        assert!(matches!(cmd, Some(ChatCommand::History)));
    }

    #[test]
    fn test_parse_switch_command() {
        let cmd = parse_command("/switch abc-123");
        assert!(matches!(cmd, Some(ChatCommand::Switch(id)) if id == "abc-123"));
    }

    #[test]
    fn test_parse_read_command() {
        let cmd = parse_command("/read src/main.rs");
        assert!(matches!(cmd, Some(ChatCommand::Read(path)) if path == "src/main.rs"));
    }

    #[test]
    fn test_parse_list_command() {
        let cmd = parse_command("/list");
        assert!(matches!(cmd, Some(ChatCommand::List(None))));

        let cmd = parse_command("/list src/");
        assert!(matches!(cmd, Some(ChatCommand::List(Some(path))) if path == "src/"));
    }

    #[test]
    fn test_parse_exit_command() {
        let cmd = parse_command("/exit");
        assert!(matches!(cmd, Some(ChatCommand::Exit)));

        let cmd = parse_command("/quit");
        assert!(matches!(cmd, Some(ChatCommand::Exit)));

        let cmd = parse_command("/q");
        assert!(matches!(cmd, Some(ChatCommand::Exit)));
    }

    #[test]
    fn test_parse_help_command() {
        let cmd = parse_command("/help");
        assert!(matches!(cmd, Some(ChatCommand::Help)));

        let cmd = parse_command("/?");
        assert!(matches!(cmd, Some(ChatCommand::Help)));
    }

    #[test]
    fn test_parse_regular_input() {
        let cmd = parse_command("hello world");
        assert!(cmd.is_none());
    }

    #[test]
    fn test_parse_unknown_command() {
        let cmd = parse_command("/unknown");
        assert!(cmd.is_none());
    }

    #[test]
    fn test_parse_command_case_insensitive() {
        let cmd = parse_command("/NEW");
        assert!(matches!(cmd, Some(ChatCommand::New)));

        let cmd = parse_command("/EXIT");
        assert!(matches!(cmd, Some(ChatCommand::Exit)));
    }

    // Tests using ChatSession from arkavo-protocol
    #[test]
    fn test_chat_session_context_mode() {
        use arkavo_protocol::A2aClient;

        let client = A2aClient::new();
        let mut session = ChatSession::from_client(client);

        assert_eq!(session.context_mode(), ContextMode::Auto);

        let result = session.cmd_context("on");
        assert!(matches!(result, CommandResult::Success(_)));
        assert_eq!(session.context_mode(), ContextMode::On);
    }

    #[test]
    fn test_chat_session_pending_context() {
        use arkavo_protocol::A2aClient;

        let client = A2aClient::new();
        let mut session = ChatSession::from_client(client);

        // Read Cargo.toml
        let result = session.cmd_read("Cargo.toml");
        assert!(matches!(result, CommandResult::Output(_)));
        assert_eq!(session.pending_context().len(), 1);
    }

    #[test]
    fn test_chat_session_help() {
        use arkavo_protocol::A2aClient;

        let client = A2aClient::new();
        let session = ChatSession::from_client(client);
        let result = session.cmd_help();

        match result {
            CommandResult::Output(text) => {
                assert!(text.contains("/new"));
                assert!(text.contains("/clear"));
            }
            _ => panic!("Expected Output"),
        }
    }

    #[tokio::test]
    async fn test_execute_command_programmatic() {
        use arkavo_protocol::A2aClient;

        let client = A2aClient::new();
        let mut session = ChatSession::from_client(client);

        // Execute commands programmatically (like an agent coder would)
        let result = execute_command(&mut session, "context", Some("on")).await;
        assert!(matches!(result, CommandResult::Success(_)));

        let result = execute_command(&mut session, "help", None).await;
        assert!(matches!(result, CommandResult::Output(_)));

        let result = execute_command(&mut session, "list", Some(".")).await;
        assert!(matches!(result, CommandResult::Output(_)));
    }
}
