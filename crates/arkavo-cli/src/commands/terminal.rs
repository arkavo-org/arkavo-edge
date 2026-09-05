use crate::mcp_integration::McpConnection;
use arkavo_llm::{LlmClient, Message};
use arkavo_memory::storage::MemoryStorage;
use arkavo_session::conversation::ConversationManager;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::runtime::Runtime;

#[cfg(all(unix, feature = "mcp-tools"))]
use arkavo_mcp_tools::ToolRegistry;
#[cfg(all(unix, feature = "mcp-tools"))]
use arkavo_router::Router;

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

#[cfg(all(unix, feature = "mcp-tools"))]
fn box_error<E: std::error::Error + Send + Sync + 'static>(
    e: E,
) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(e)
}

fn initialize_mcp_connection(print_mode: bool) -> Option<McpConnection> {
    // Try in-process MCP first on macOS with mcp-tools feature
    #[cfg(all(target_os = "macos", feature = "mcp-macos"))]
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
        .unwrap_or_else(|| "qwen3".to_string());

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

    // Initialize router and tool registry for quality gate
    #[cfg(all(unix, feature = "mcp-tools"))]
    let (router, tool_registry) = {
        let router = runtime.block_on(Router::new()).ok();
        let registry = mcp_client.as_ref().and_then(|_| {
            let storage = runtime
                .block_on(arkavo_memory::MemoryStorage::new())
                .ok()
                .map(std::sync::Arc::new)?;
            Some(ToolRegistry::from_mcp_or_default(None, storage))
        });
        (router, registry)
    };

    // Spawn LLM processing task with router quality gate integration
    let llm_handle = runtime.spawn(async move {
        if SHOW_DEBUG.load(Ordering::Relaxed) {
            eprintln!("[LLM Task] Started with router quality gate...");
        }
        while let Some(user_input) = ui_rx.recv().await {
            if SHOW_DEBUG.load(Ordering::Relaxed) {
                eprintln!("[LLM Task] Received user input: {user_input}");
            }
            // Process the user input with LLM
            let user_message = Message::user(user_input.clone());
            messages_clone.push(user_message);

            // Try router-based routing with Thompson Sampling (Unix + mcp-tools)
            // Router provides: cost optimization, quality validation, TS learning
            #[cfg(all(unix, feature = "mcp-tools"))]
            let route_result: std::result::Result<
                Option<arkavo_llm::ProviderResponse>,
                Box<dyn std::error::Error + Send + Sync>,
            > = match (router.as_ref(), &client_clone) {
                (Some(router), _) => {
                    let task_desc = user_input.clone();
                    match router
                        .route_with_tools(
                            &task_desc,
                            messages_clone.clone(),
                            tool_registry.as_ref(),
                        )
                        .await
                    {
                        Ok(response) => Ok(Some(response)),
                        Err(e) => Err(box_error(e)),
                    }
                }
                (None, _) => Ok(None),
            };

            // Determine if we got a router response or need to fall back to streaming
            #[cfg(all(unix, feature = "mcp-tools"))]
            let use_streaming_fallback = match &route_result {
                Ok(Some(_)) => false,
                Ok(None) => true,
                Err(error) => {
                    let _ = llm_tx.send(format!("Error: {error}")).await;
                    continue;
                }
            };

            #[cfg(not(all(unix, feature = "mcp-tools")))]
            let route_result: std::result::Result<
                Option<arkavo_llm::ProviderResponse>,
                Box<dyn std::error::Error + Send + Sync>,
            > = Ok(None);
            #[cfg(not(all(unix, feature = "mcp-tools")))]
            let use_streaming_fallback = true;

            if !use_streaming_fallback {
                // Router returned a complete response — send as single chunk
                if let Ok(Some(response)) = route_result {
                    let content = response.content.clone();
                    if SHOW_DEBUG.load(Ordering::Relaxed) {
                        eprintln!("[LLM Task] Response via Router (Thompson Sampling)");
                    }
                    let _ = llm_tx.send("<<STREAM_START>>".to_string()).await;
                    let _ = llm_tx.send(content.clone()).await;
                    let _ = llm_tx.send("<<STREAM_END>>".to_string()).await;

                    // Add assistant response to messages
                    messages_clone.push(response.as_assistant_message());
                }
            } else {
                // Fallback to direct LLM streaming
                let stream_result = client_clone
                    .stream(messages_clone.clone())
                    .await
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>);

                match stream_result {
                    Ok(mut stream) => {
                        if SHOW_DEBUG.load(Ordering::Relaxed) {
                            eprintln!("[LLM Task] Streaming via Direct LLM");
                        }
                        let mut full_response = String::new();
                        let mut response_items = Vec::new();

                        let _ = llm_tx.send("<<STREAM_START>>".to_string()).await;

                        while let Some(chunk) = tokio_stream::StreamExt::next(&mut stream).await {
                            match chunk {
                                Ok(response) => {
                                    full_response.push_str(&response.content);
                                    if let Err(e) = llm_tx.send(response.content).await {
                                        eprintln!("[LLM Task] Failed to send chunk: {e}");
                                        break;
                                    }
                                    if response.done {
                                        response_items = response.response_items;
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

                        let _ = llm_tx.send("<<STREAM_END>>".to_string()).await;

                        let mut assistant = Message::assistant(full_response);
                        assistant.response_items = response_items;
                        messages_clone.push(assistant);

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

        // Select model based on parameter — resolve via ModelChoice for single source of truth
        use arkavo_router::decision::ModelChoice;
        let model_choice = match model_name {
            "gemma-2-2b" | "gemma-2b" => None, // legacy, no ModelChoice variant
            _ => ModelChoice::from_name(model_name),
        };

        let (model_repo, model_file, display_name) = if let Some(mc) = &model_choice
            && let Some(repo) = mc.repo_id()
            && let Some(file) = mc.gguf_filename()
        {
            (repo, file, mc.name())
        } else if model_name == "gemma-2-2b" || model_name == "gemma-2b" {
            (
                "bartowski/gemma-2-2b-it-GGUF",
                "gemma-2-2b-it-Q4_K_M.gguf",
                "gemma-2-2b-it",
            )
        } else {
            // Default to smallest TØRG-compatible model
            (
                ModelChoice::LocalQwen3.repo_id().unwrap(),
                ModelChoice::LocalQwen3.gguf_filename().unwrap(),
                ModelChoice::LocalQwen3.name(),
            )
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
