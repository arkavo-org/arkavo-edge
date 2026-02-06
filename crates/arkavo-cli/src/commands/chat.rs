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

    // A2A mode is the default (and only) chat mode
    let prompt = args
        .windows(2)
        .find(|w| w[0] == "--prompt" || w[0] == "--print")
        .map(|w| w[1].clone());
    execute_a2a_chat(prompt.as_deref())
}

fn print_usage() {
    println!("Start interactive chat with LLM\n");
    println!("USAGE:");
    println!("    arkavo chat                     Interactive chat mode");
    println!("    arkavo chat --prompt \"query\"    One-shot query\n");
    println!("EXAMPLES:");
    println!("    arkavo chat");
    println!("    arkavo chat --prompt \"What is 2+2?\"\n");
    println!("OPTIONS:");
    println!("    --prompt <TEXT>       One-shot query (exits after response)");
    println!("    --debug               Show debug output");
    println!("    -h, --help            Show this help\n");
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
#[allow(clippy::disallowed_methods)]
fn execute_a2a_chat(prompt: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = get_or_create_runtime();

    runtime.block_on(async {
        // Initialize router
        let router = arkavo_router::Router::new().await?;

        // Initialize memory storage for tool registry
        let storage = Arc::new(arkavo_memory::storage::MemoryStorage::new().await?);

        // Create tool registry with built-in tools (time, filesystem, etc.)
        let tool_registry = arkavo_mcp_tools::ToolRegistry::new(storage);

        // Create ChatSession (wraps A2aClient)
        let mut session = ChatSession::new(Arc::new(router), Some(Arc::new(tool_registry))).await?;

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

/// Process streaming response
async fn process_stream(
    rx: &mut tokio::sync::mpsc::Receiver<arkavo_protocol::types::MessageDelta>,
) {
    use arkavo_protocol::types::MessageDeltaContent;

    while let Some(delta) = rx.recv().await {
        match &delta.delta {
            MessageDeltaContent::Text { text } => {
                print!("{text}");
                let _ = io::stdout().flush();
            }
            MessageDeltaContent::ToolCall { name, .. } => {
                if let Some(name) = name {
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
                println!();
                break;
            }
            MessageDeltaContent::Error { message, .. } => {
                eprintln!("\n[Error: {message}]");
                break;
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
