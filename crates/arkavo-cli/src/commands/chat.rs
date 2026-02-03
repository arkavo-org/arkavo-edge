use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::runtime::Runtime;

// Global flag to control whether to show debug messages
// Set via --debug command line flag
pub(crate) static SHOW_DEBUG: AtomicBool = AtomicBool::new(false);

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
        println!("    /exit, /quit          Exit chat");
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

/// Execute A2A chat mode using in-process A2aClient with router
#[allow(clippy::disallowed_methods)]
fn execute_a2a_chat(prompt: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    use arkavo_protocol::A2aClient;

    let runtime = get_or_create_runtime();

    runtime.block_on(async {
        // Initialize router
        let router = arkavo_router::Router::new().await?;

        // Initialize memory storage for tool registry
        let storage = Arc::new(arkavo_memory::storage::MemoryStorage::new().await?);

        // Create tool registry with built-in tools (time, filesystem, etc.)
        let tool_registry = arkavo_mcp_tools::ToolRegistry::new(storage);

        // Create A2A client with router
        let mut client = A2aClient::with_router(Arc::new(router), Some(Arc::new(tool_registry)));

        let session_id = client.open_session().await?;

        if std::env::var("ARKAVO_DEBUG").is_ok() {
            eprintln!("[A2A] Session: {}", &session_id[..8.min(session_id.len())]);
        }

        // One-shot mode
        if let Some(prompt) = prompt {
            let mut rx = client.send_message(prompt).await?;
            process_a2a_stream(&mut rx).await;
            client.close_session().await?;
            return Ok(());
        }

        // Interactive REPL
        println!("A2A Chat Mode (type /exit to quit)");
        loop {
            let input = read_a2a_user_input()?;
            if input == "/exit" || input == "/quit" {
                break;
            }

            let mut rx = client.send_message(&input).await?;
            process_a2a_stream(&mut rx).await;
        }

        client.close_session().await?;
        Ok(())
    })
}

/// Process streaming response from A2A client
async fn process_a2a_stream(
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

/// Read user input for A2A REPL
fn read_a2a_user_input() -> Result<String, Box<dyn std::error::Error>> {
    print!("> ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}
