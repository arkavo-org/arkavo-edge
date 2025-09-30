use crate::conversation_manager::ConversationManager;
use crate::mcp_integration::McpConnection;
use arkavo_llm::{LlmClient, Message};
use arkavo_memory::storage::MemoryStorage;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::runtime::Runtime;

// Global flag to control whether to show debug messages
// Set via ARKAVO_DEBUG_CHAT=1 environment variable
static SHOW_DEBUG: AtomicBool = AtomicBool::new(false);

// Global runtime to prevent multiple runtime creation issues
static RUNTIME: std::sync::OnceLock<Runtime> = std::sync::OnceLock::new();

fn get_or_create_runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("arkavo-terminal-worker")
            .thread_stack_size(3 * 1024 * 1024)
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime")
    })
}

fn initialize_mcp_connection(print_mode: bool) -> Option<McpConnection> {
    // Try in-process MCP first on macOS with test-harness feature
    #[cfg(all(target_os = "macos", feature = "test-harness"))]
    {
        let result = McpConnection::new_in_process();
        match result {
            Ok(mcp) => {
                if !print_mode {
                    eprintln!("✓ In-process MCP tools available");
                    if let Ok(tools) = mcp.list_tools() {
                        eprintln!("  Available tools: {} tools", tools.len());
                    }
                }
                return Some(mcp);
            }
            Err(e) => {
                if !print_mode {
                    eprintln!("⚠ In-process MCP not available: {e}");
                }
            }
        }
    }

    // Try external MCP if configured
    let mcp_url = std::env::var("MCP_URL").ok();

    if mcp_url.is_none() {
        if !print_mode {
            eprintln!("ℹ MCP not configured - using LLM-only mode");
        }
        return None;
    }

    match McpConnection::new_external(mcp_url) {
        Ok(client) => {
            if !print_mode {
                eprintln!("✓ External MCP connected");
                if let Ok(tools) = client.list_tools() {
                    eprintln!("  Available tools: {} tools", tools.len());
                }
            }
            Some(client)
        }
        Err(e) => {
            if !print_mode {
                eprintln!("ℹ MCP server not available - using LLM-only mode: {e}");
            }
            None
        }
    }
}

#[allow(clippy::disallowed_methods)]
pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // Check for --debug flag in arguments
    if args.iter().any(|arg| arg == "--debug") {
        SHOW_DEBUG.store(true, Ordering::Relaxed);
    }

    // Parse model selection
    let model_name = args
        .windows(2)
        .find(|w| w[0] == "--model")
        .map(|w| w[1].clone())
        .unwrap_or_else(|| "gemma-3-270m".to_string());

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

    // Initialize storage and conversation manager
    let storage = Arc::new(runtime.block_on(MemoryStorage::new())?);
    let mut conversation_manager = ConversationManager::new(storage)?;

    // Initialize LLM client
    let client = runtime.block_on(initialize_llm_client(
        false,
        &model_name,
        temperature,
        top_p,
        top_k,
        max_tokens,
        seed,
    ))?;

    println!("Starting Terminal UI session...");
    println!("LLM Provider: {}", client.provider_name());

    // Try to restore last session
    if let Ok(Some(session_id)) = runtime.block_on(conversation_manager.restore_last_session()) {
        println!(
            "Restored previous conversation (session: {})",
            &session_id.to_string()[..8]
        );
    } else {
        // Start new session
        let _ = runtime.block_on(conversation_manager.start_session(client.provider_name()))?;
    }

    // Initialize prompt override directory if needed
    let _ = crate::prompt_loader::init_prompt_override_dir();

    // Initialize MCP client
    let mcp_client: Option<McpConnection> = initialize_mcp_connection(false);

    // Get initial messages from conversation manager
    let system_prompt = crate::prompt_loader::load_terminal_system_prompt(
        mcp_client.is_some(),
        None, // MCP info will be added later if available
    );
    let system_message = Message::system(&system_prompt);
    let messages =
        runtime.block_on(conversation_manager.get_context_messages(Some(system_message)))?;

    // Create channels for communication between TUI and LLM
    let (ui_tx, mut ui_rx) = tokio::sync::mpsc::channel::<String>(100);
    let (llm_tx, llm_rx) = tokio::sync::mpsc::channel::<String>(100);

    // Clone necessary components for the TUI task
    let client = Arc::new(client);
    let client_clone: Arc<LlmClient> = Arc::clone(&client);
    let mut messages_clone = messages;

    // Spawn LLM processing task
    let llm_handle = runtime.spawn(async move {
        if SHOW_DEBUG.load(Ordering::Relaxed) {
            eprintln!("[LLM Task] Started, waiting for messages...");
        }
        while let Some(user_input) = ui_rx.recv().await {
            if SHOW_DEBUG.load(Ordering::Relaxed) {
                eprintln!("[LLM Task] Received user input: {user_input}");
            }
            // Process the user input with LLM
            let user_message = Message::user(user_input.clone());
            messages_clone.push(user_message);

            // Get streaming response from LLM
            match client_clone.stream(messages_clone.clone()).await {
                Ok(mut stream) => {
                    let mut full_response = String::new();

                    // Send start streaming signal
                    let _ = llm_tx.send("<<STREAM_START>>".to_string()).await;

                    // Stream the response
                    while let Some(chunk) = tokio_stream::StreamExt::next(&mut stream).await {
                        match chunk {
                            Ok(response) => {
                                full_response.push_str(&response.content);
                                if let Err(e) = llm_tx.send(response.content).await {
                                    eprintln!("[LLM Task] Failed to send chunk: {e}");
                                    break;
                                }
                                if response.done {
                                    break;
                                }
                            }
                            Err(e) => {
                                eprintln!("[LLM Task] Stream error: {e}");
                                let _ = llm_tx.send(format!("Error: {e}")).await;
                                break;
                            }
                        }
                    }

                    // Send end streaming signal
                    let _ = llm_tx.send("<<STREAM_END>>".to_string()).await;

                    // Add assistant response to messages
                    messages_clone.push(Message::assistant(full_response));

                    if SHOW_DEBUG.load(Ordering::Relaxed) {
                        eprintln!(
                            "[LLM Task] Response complete, total messages: {}",
                            messages_clone.len()
                        );
                    }
                }
                Err(e) => {
                    eprintln!("[LLM Task] Error: {e}");
                    let _ = llm_tx.send(format!("Error: {e}")).await;
                }
            }
            if SHOW_DEBUG.load(Ordering::Relaxed) {
                eprintln!("[LLM Task] Waiting for next message...");
            }
        }
        if SHOW_DEBUG.load(Ordering::Relaxed) {
            eprintln!("[LLM Task] Channel closed, exiting...");
        }
    });

    // Run the Terminal UI with communication channels
    let tui_result =
        runtime.block_on(async { arkavo_terminal::run_with_string_channels(ui_tx, llm_rx).await });

    // Clean up
    llm_handle.abort();

    tui_result.map_err(std::convert::Into::into)
}

async fn initialize_llm_client(
    print_mode: bool,
    #[cfg_attr(not(feature = "llama-cpp"), allow(unused_variables))] model_name: &str,
    #[cfg_attr(not(feature = "llama-cpp"), allow(unused_variables))] temperature: f32,
    #[cfg_attr(not(feature = "llama-cpp"), allow(unused_variables))] top_p: f32,
    #[cfg_attr(not(feature = "llama-cpp"), allow(unused_variables))] top_k: i32,
    #[cfg_attr(not(feature = "llama-cpp"), allow(unused_variables))] max_tokens: u32,
    #[cfg_attr(not(feature = "llama-cpp"), allow(unused_variables))] seed: u32,
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
        println!("Ollama not available. Using local model with llama.cpp...");
    }

    // Fallback to local llama.cpp
    #[cfg(feature = "llama-cpp")]
    {
        use hf_hub::api::tokio::Api;

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
                    println!("Downloading model... this may take a while.");
                }
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
