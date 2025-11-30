pub mod app;
pub mod benchmark;
pub mod event;
pub mod helix;
pub mod model_manager;
pub mod multi_terminal;
pub mod renderer;
pub mod telemetry;
pub mod ui;
pub mod vim;

#[cfg(test)]
mod tests;

use anyhow::Result;
#[cfg(any(feature = "llm-remote", feature = "llama-cpp"))]
use std::sync::Arc;
use tokio::sync::mpsc;

pub use app::App;
pub use event::{AppEvent, EventHandler};
pub use multi_terminal::{MultiTerminalManager, TaskType, TerminalSpawnConfig};
pub use renderer::{DiffRenderer, RenderMetrics, Renderable};

#[derive(Debug, Clone)]
pub enum ChatMessage {
    UserInput(String),
    AssistantResponse(String),
    SystemMessage(String),
    Error(String),
}

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub task_id: uuid::Uuid,
    pub model_name: String,
    pub prompt: String,
}

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub task_id: uuid::Uuid,
    pub model_name: String,
    pub content: String,
    pub is_streaming: bool,
    pub is_complete: bool,
    pub error: Option<String>,
    pub mcp_status: Option<McpStatusUpdate>,
}

#[derive(Debug, Clone)]
pub struct McpStatusUpdate {
    pub available: bool,
    pub error_message: Option<String>,
    pub tool_count: usize,
}

pub struct TerminalContext {
    pub message_tx: mpsc::Sender<ChatMessage>,
    pub message_rx: mpsc::Receiver<ChatMessage>,
}

/// Run the terminal UI application
///
/// # Panics
///
/// Panics if the MCP client is Some but becomes None unexpectedly
pub async fn run() -> Result<()> {
    // Create channels for LLM communication
    #[cfg(any(feature = "llm-remote", feature = "llama-cpp"))]
    let (ui_tx, ui_rx) = mpsc::channel::<LlmRequest>(100);
    #[cfg(not(any(feature = "llm-remote", feature = "llama-cpp")))]
    let (ui_tx, _ui_rx) = mpsc::channel::<LlmRequest>(100);

    #[cfg(any(feature = "llm-remote", feature = "llama-cpp"))]
    let (llm_tx, llm_rx) = mpsc::channel::<LlmResponse>(100);
    #[cfg(not(any(feature = "llm-remote", feature = "llama-cpp")))]
    let (_llm_tx, llm_rx) = mpsc::channel::<LlmResponse>(100);

    // Spawn LLM handler task with proper Ollama integration
    #[cfg(any(feature = "llm-remote", feature = "llama-cpp"))]
    {
        let mut ui_rx = ui_rx;
        let llm_tx = llm_tx.clone();
        tokio::spawn(async move {
            use arkavo_llm::Message;
            use arkavo_mcp_tools::mcp_connection::McpConnection;
            use tokio_stream::StreamExt;

            // Initialize LLM client using the same logic as chat command
            let client = match initialize_llm_client().await {
                Ok(client) => std::sync::Arc::new(client),
                Err(e) => {
                    eprintln!("Failed to initialize LLM client: {e}");
                    return;
                }
            };

            // Only log in debug builds
            #[cfg(debug_assertions)]
            eprintln!(
                "Terminal UI connected to LLM provider: {}",
                client.provider_name()
            );

            // Initialize MCP connection and report status
            let mcp_client = McpConnection::new().ok();

            let mcp_status = if let Some(ref client) = mcp_client {
                let tool_count = client.list_tools().len();

                McpStatusUpdate {
                    available: true,
                    error_message: None,
                    tool_count,
                }
            } else {
                McpStatusUpdate {
                    available: false,
                    error_message: Some("MCP tools not available".to_string()),
                    tool_count: 0,
                }
            };

            // Send MCP status update through LLM channel
            let _ = llm_tx
                .send(LlmResponse {
                    task_id: uuid::Uuid::new_v4(),
                    model_name: "system".to_string(),
                    content: String::new(),
                    is_streaming: false,
                    is_complete: true,
                    error: None,
                    mcp_status: Some(mcp_status),
                })
                .await;

            // Build MCP tools information for system prompt
            let mcp_info = if let Some(ref client) = mcp_client {
                let tool_names = client.list_tools();
                if tool_names.is_empty() {
                    "\n\nMCP Integration: Enabled\nNo tools available yet.".to_string()
                } else {
                    let mut tool_info =
                        String::from("\n\nMCP Integration: Enabled\n\nAvailable MCP tools:\n");

                    // Group tools by category
                    let mut device_tools = Vec::new();
                    let mut ui_tools = Vec::new();
                    let mut git_tools = Vec::new();
                    let mut memory_tools = Vec::new();
                    let mut other_tools = Vec::new();

                    for tool_name in &tool_names {
                        let tool_desc = format!("- @{tool_name}");

                        if tool_name.contains("device") || tool_name.contains("simulator") {
                            device_tools.push(tool_desc);
                        } else if tool_name.contains("ui_") || tool_name.contains("screen") {
                            ui_tools.push(tool_desc);
                        } else if tool_name.contains("git_") {
                            git_tools.push(tool_desc);
                        } else if tool_name.contains("memory")
                            || tool_name == "store_memory"
                            || tool_name == "search_memory"
                        {
                            memory_tools.push(tool_desc);
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

                    if !git_tools.is_empty() {
                        tool_info.push_str("\n\nGit Tools:\n");
                        tool_info.push_str(&git_tools.join("\n"));
                    }

                    if !memory_tools.is_empty() {
                        tool_info.push_str("\n\nMemory Tools:\n");
                        tool_info.push_str(&memory_tools.join("\n"));
                    }

                    if !other_tools.is_empty() {
                        tool_info.push_str("\n\nOther Tools:\n");
                        tool_info.push_str(&other_tools.join("\n"));
                    }

                    tool_info
                }
            } else {
                "\n\nMCP Integration: Disabled".to_string()
            };

            // Try to read .arkavo/AGENTS.md first, then AGENTS.md, then CLAUDE.md
            let agents_md_content = if std::path::Path::new(".arkavo/AGENTS.md").exists() {
                std::fs::read_to_string(".arkavo/AGENTS.md").ok()
            } else {
                match std::fs::read_to_string("AGENTS.md") {
                    Ok(content) => Some(content),
                    Err(_) => {
                        // Try CLAUDE.md as fallback
                        std::fs::read_to_string("CLAUDE.md").ok()
                    }
                }
            };

            // System prompt with MCP tools information
            let system_prompt = if let Some(agents_content) = agents_md_content {
                format!(
                    "{agents_content}\n\nMCP Integration: You have access to MCP tools for various operations including Git, device management, and UI interaction. \
                When the user asks you to perform actions, you can use these tools by including @toolname commands in your response.\n\
                \nTo invoke an MCP tool, use the format: @toolname {{arguments}} or @toolname plain text arguments\
                \nFor example: @git_status {{}} or @device_management {{\"action\": \"list\"}}\
                {mcp_info}"
                )
            } else {
                format!(
                    "You are an AI assistant working in the Arkavo Terminal UI. \
                You have access to MCP tools for various operations including Git, device management, and UI interaction. \
                When the user asks you to perform actions, you can use these tools by including @toolname commands in your response.\n\
                \nTo invoke an MCP tool, use the format: @toolname {{arguments}} or @toolname plain text arguments\
                \nFor example: @git_status {{}} or @device_management {{\"action\": \"list\"}}\
                {mcp_info}"
                )
            };

            // Keep conversation context with system message
            let mut messages = vec![Message::system(&system_prompt)];

            while let Some(request) = ui_rx.recv().await {
                let llm_tx = llm_tx.clone();
                let mut messages_clone = messages.clone();

                // Add user message to context
                let user_message = Message::user(&request.prompt);
                messages_clone.push(user_message.clone());

                // Create a channel to receive the assistant's response
                let (response_tx, mut response_rx) = tokio::sync::mpsc::channel::<String>(1);

                // Parse the model name to extract server and actual model
                let (server_url, actual_model) = if let Some((server_prefix, model)) =
                    request.model_name.split_once('/')
                {
                    // Model has server prefix - need to resolve the URL
                    let url = if server_prefix == "localhost" {
                        "http://localhost:11434".to_string()
                    } else if server_prefix.starts_with("server") {
                        // Look up saved server configuration from memory storage
                        if let Ok(storage) = arkavo_memory::storage::MemoryStorage::new().await {
                            if let Ok(all_configs) = storage
                                .search("arkavo_ollama_server_config", 20, Some("config"))
                                .await
                            {
                                // No need to filter since we searched for the specific type
                                let ollama_configs: Vec<_> = all_configs
                                    .into_iter()
                                    .filter(|config| {
                                        config.memory.content != "CLEARED"
                                            && config.memory.content != "http://localhost:11434"
                                    })
                                    .collect();

                                #[cfg(debug_assertions)]
                                {
                                    eprintln!(
                                        "[LLM] Found {} Ollama server configs:",
                                        ollama_configs.len()
                                    );
                                    for (i, config) in ollama_configs.iter().enumerate() {
                                        eprintln!(
                                            "[LLM]   server{} -> {}",
                                            i + 1,
                                            config.memory.content
                                        );
                                    }
                                }

                                // Extract server number from prefix (e.g., "server1" -> 1)
                                if let Some(num_str) = server_prefix.strip_prefix("server") {
                                    if let Ok(idx) = num_str.parse::<usize>() {
                                        if idx > 0 && idx <= ollama_configs.len() {
                                            let server_url =
                                                ollama_configs[idx - 1].memory.content.clone();
                                            #[cfg(debug_assertions)]
                                            eprintln!(
                                                "[LLM] Resolved {server_prefix} to URL: {server_url}"
                                            );
                                            server_url
                                        } else {
                                            #[cfg(debug_assertions)]
                                            eprintln!(
                                                "[LLM] Server index {} out of range (have {} servers)",
                                                idx,
                                                ollama_configs.len()
                                            );
                                            "http://localhost:11434".to_string()
                                        }
                                    } else {
                                        "http://localhost:11434".to_string()
                                    }
                                } else {
                                    "http://localhost:11434".to_string()
                                }
                            } else {
                                "http://localhost:11434".to_string()
                            }
                        } else {
                            "http://localhost:11434".to_string()
                        }
                    } else {
                        // Unknown prefix, use localhost as fallback
                        "http://localhost:11434".to_string()
                    };
                    (url, model.to_string())
                } else {
                    // No server prefix, use default
                    (
                        "http://localhost:11434".to_string(),
                        request.model_name.clone(),
                    )
                };

                // Only log in debug builds
                #[cfg(debug_assertions)]
                eprintln!("[LLM] Using server: {server_url} with model: {actual_model}");

                // Create a new Ollama client with the specific model and server
                let model_specific_client =
                    arkavo_llm::LlmClient::new(Box::new(arkavo_llm::ollama::OllamaClient::new(
                        Some(server_url.clone()),
                        Some(actual_model.clone()),
                    )));

                // Clone MCP client for this task
                let task_mcp_client = mcp_client.clone();
                let provider_name = client.provider_name().to_string();

                // Spawn a task for each request
                tokio::spawn(async move {
                    // Get streaming response from LLM with the user-selected model
                    match model_specific_client.stream(messages_clone.clone()).await {
                        Ok(mut stream) => {
                            // Send start streaming signal
                            let _ = llm_tx
                                .send(LlmResponse {
                                    task_id: request.task_id,
                                    model_name: request.model_name.clone(),
                                    content: String::new(),
                                    is_streaming: true,
                                    is_complete: false,
                                    error: None,
                                    mcp_status: None,
                                })
                                .await;

                            let mut full_response = String::new();

                            while let Some(chunk_result) = stream.next().await {
                                match chunk_result {
                                    Ok(chunk) => {
                                        if !chunk.content.is_empty() {
                                            full_response.push_str(&chunk.content);
                                            // Send each chunk as it arrives
                                            let _ = llm_tx
                                                .send(LlmResponse {
                                                    task_id: request.task_id,
                                                    model_name: request.model_name.clone(),
                                                    content: chunk.content,
                                                    is_streaming: true,
                                                    is_complete: false,
                                                    error: None,
                                                    mcp_status: None,
                                                })
                                                .await;
                                        }
                                    }
                                    Err(e) => {
                                        let _ = llm_tx
                                            .send(LlmResponse {
                                                task_id: request.task_id,
                                                model_name: request.model_name.clone(),
                                                content: String::new(),
                                                is_streaming: false,
                                                is_complete: true,
                                                error: Some(format!("Stream error: {e}")),
                                                mcp_status: None,
                                            })
                                            .await;
                                        break;
                                    }
                                }
                            }

                            // Check for and execute any @tool calls in the response
                            if let Some(ref mcp) = task_mcp_client {
                                let tool_calls = extract_tool_calls(&full_response);
                                for (tool_name, args_str) in tool_calls {
                                    // Parse arguments
                                    let args = if args_str.trim().starts_with('{') {
                                        serde_json::from_str(&args_str).unwrap_or_else(
                                            |_| serde_json::json!({"prompt": args_str}),
                                        )
                                    } else {
                                        serde_json::json!({"prompt": args_str})
                                    };

                                    // Execute tool
                                    let tool_response = match mcp.call_tool(
                                        &tool_name,
                                        args,
                                        &provider_name,
                                    ) {
                                        Ok(result) => {
                                            // Extract result text
                                            let result_text = if let Some(result_obj) =
                                                result.get("result")
                                            {
                                                if let Some(text) = result_obj.as_str() {
                                                    text.to_string()
                                                } else {
                                                    serde_json::to_string_pretty(&result_obj)
                                                        .unwrap_or_else(|_| result_obj.to_string())
                                                }
                                            } else {
                                                serde_json::to_string_pretty(&result)
                                                    .unwrap_or_else(|_| result.to_string())
                                            };

                                            LlmResponse {
                                                task_id: request.task_id,
                                                model_name: request.model_name.clone(),
                                                content: format!(
                                                    "\n\n[Tool Result - @{tool_name}]:\n{result_text}"
                                                ),
                                                is_streaming: false,
                                                is_complete: false,
                                                error: None,
                                                mcp_status: None,
                                            }
                                        }
                                        Err(e) => LlmResponse {
                                            task_id: request.task_id,
                                            model_name: request.model_name.clone(),
                                            content: format!(
                                                "\n\n[Tool Error - @{tool_name}]: {e}"
                                            ),
                                            is_streaming: false,
                                            is_complete: false,
                                            error: None,
                                            mcp_status: None,
                                        },
                                    };

                                    let _ = llm_tx.send(tool_response).await;
                                }
                            }

                            // Send completion signal
                            let _ = llm_tx
                                .send(LlmResponse {
                                    task_id: request.task_id,
                                    model_name: request.model_name,
                                    content: String::new(),
                                    is_streaming: true,
                                    is_complete: true,
                                    error: None,
                                    mcp_status: None,
                                })
                                .await;

                            // Send the full response back to be added to conversation history
                            let _ = response_tx.send(full_response).await;
                        }
                        Err(e) => {
                            // Provide more informative error messages
                            let error_msg = if e.to_string().contains("404")
                                || e.to_string().contains("not found")
                            {
                                format!(
                                    "Model '{actual_model}' not found on server {server_url}. Please check available models for this server."
                                )
                            } else {
                                format!("Failed to get LLM response from {server_url}: {e}")
                            };

                            let _ = llm_tx
                                .send(LlmResponse {
                                    task_id: request.task_id,
                                    model_name: request.model_name,
                                    content: String::new(),
                                    is_streaming: false,
                                    is_complete: true,
                                    error: Some(error_msg),
                                    mcp_status: None,
                                })
                                .await;
                        }
                    }
                });

                // Update context with user message
                messages.push(user_message);

                // Wait for assistant response and add to context
                if let Some(assistant_response) = response_rx.recv().await {
                    messages.push(Message::assistant(&assistant_response));
                }
            }
        });
    }

    run_with_channels(ui_tx, llm_rx).await
}

#[cfg(any(feature = "llm-remote", feature = "llama-cpp"))]
async fn initialize_llm_client() -> Result<arkavo_llm::LlmClient> {
    use arkavo_llm::{LlmClient, Message};
    use arkavo_memory::storage::MemoryStorage;
    use std::sync::Arc;

    // Initialize memory storage to check for saved configuration
    let storage = Arc::new(MemoryStorage::new().await?);

    // Try to find saved Ollama server configuration
    let saved_config = storage
        .search("arkavo_ollama_server_config", 10, Some("config"))
        .await?;

    // Find a valid configuration (not cleared)
    let valid_config = saved_config
        .into_iter()
        .find(|c| c.memory.content != "CLEARED" && c.memory.content.starts_with("http"));

    if let Some(config) = valid_config {
        // Use saved configuration
        let server_url = &config.memory.content;
        unsafe {
            std::env::set_var("OLLAMA_BASE_URL", server_url);
        }

        // Try to connect with saved URL
        if let Ok(client) = LlmClient::from_env() {
            let test_message = vec![Message::user("ping")];
            if client.complete(test_message).await.is_ok() {
                return Ok(client);
            }
        }
    }

    // Try default localhost first
    match LlmClient::from_env() {
        Ok(client) => {
            // Test if the client can connect
            let test_message = vec![Message::user("ping")];
            match client.complete(test_message).await {
                Ok(_) => Ok(client),
                Err(_) => {
                    // Prompt for configuration
                    prompt_for_ollama_config(storage).await
                }
            }
        }
        Err(_) => {
            // Prompt for configuration
            prompt_for_ollama_config(storage).await
        }
    }
}

#[cfg(any(feature = "llm-remote", feature = "llama-cpp"))]
async fn prompt_for_ollama_config(
    storage: Arc<arkavo_memory::storage::MemoryStorage>,
) -> Result<arkavo_llm::LlmClient> {
    use arkavo_llm::{LlmClient, Message};
    use std::io::{self, Write};

    eprintln!("⚠️  Could not connect to Ollama at localhost:11434");
    eprintln!("Please ensure Ollama is running or provide a remote server address.");
    eprintln!();
    print!("Enter Ollama server address (e.g., 192.168.1.100:11434) or press Enter to exit: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if input.is_empty() {
        return Err(anyhow::anyhow!("No Ollama server configured"));
    }

    // Ensure the URL has the correct format
    let base_url = if input.starts_with("http://") || input.starts_with("https://") {
        input.to_string()
    } else {
        format!("http://{input}")
    };

    // Set the environment variable
    unsafe {
        std::env::set_var("OLLAMA_BASE_URL", &base_url);
    }

    // Try to create client and test it
    match LlmClient::from_env() {
        Ok(client) => {
            let test_message = vec![Message::user("ping")];
            match client.complete(test_message).await {
                Ok(_) => {
                    eprintln!("✓ Connected to Ollama at {base_url}");

                    // Save configuration for future use
                    let embedding_service = arkavo_memory::embeddings::EmbeddingService::new();
                    let embedding = match embedding_service.generate_embedding(&base_url).await {
                        Ok(e) => e,
                        Err(_) => vec![0.0; 384], // Default embedding size
                    };

                    let memory = arkavo_memory::models::Memory {
                        id: uuid::Uuid::new_v4(),
                        content: base_url.clone(),
                        metadata: Some(serde_json::json!({
                            "type": "arkavo_ollama_server_config",
                            "timestamp": chrono::Utc::now().to_rfc3339()
                        })),
                        category: Some("config".to_string()),
                        embedding,
                        created_at: chrono::Utc::now(),
                        updated_at: chrono::Utc::now(),
                    };

                    if let Err(e) = storage.store(memory).await {
                        eprintln!("Warning: Could not save configuration: {e}");
                    } else {
                        eprintln!("✓ Saved configuration for future use");
                    }

                    Ok(client)
                }
                Err(e) => Err(anyhow::anyhow!(
                    "Failed to connect to Ollama at {base_url}: {e}"
                )),
            }
        }
        Err(e) => Err(anyhow::anyhow!("Failed to create client: {e}")),
    }
}

pub async fn run_with_channels(
    ui_tx: mpsc::Sender<LlmRequest>,
    llm_rx: mpsc::Receiver<LlmResponse>,
) -> Result<()> {
    // Install panic hook to restore terminal on panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Restore terminal state
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture
        );

        // Call the original panic hook
        original_hook(panic_info);
    }));

    let mut app = App::new_with_channels(ui_tx, llm_rx);
    let result = app.run().await;

    // Restore original panic hook
    let _ = std::panic::take_hook();

    result
}

// Compatibility layer for existing string-based interface
pub async fn run_with_string_channels(
    ui_tx: mpsc::Sender<String>,
    mut llm_rx: mpsc::Receiver<String>,
) -> Result<()> {
    // Create adapter channels
    let (new_ui_tx, mut new_ui_rx) = mpsc::channel::<LlmRequest>(100);
    let (new_llm_tx, new_llm_rx) = mpsc::channel::<LlmResponse>(100);

    // Track order of requests since current protocol doesn't include task IDs
    let mut request_queue = std::collections::VecDeque::new();

    // Spawn adapter task to convert messages
    tokio::spawn(async move {
        loop {
            tokio::select! {
                // Convert outgoing LlmRequest to String
                Some(request) = new_ui_rx.recv() => {
                    // Store the request in queue
                    request_queue.push_back((request.task_id, request.model_name.clone()));

                    // Include model selection in the prompt format
                    let formatted_prompt = format!("@{} {}", request.model_name, request.prompt);
                    match ui_tx.try_send(formatted_prompt) {
                        Ok(_) => {
                            // Set a timeout for response
                            let task_id = request.task_id;
                            let model_name = request.model_name.clone();
                            let tx_clone = new_llm_tx.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                                // Check if still pending after timeout
                                let timeout_response = LlmResponse {
                                    task_id,
                                    model_name: model_name.clone(),
                                    content: String::new(),
                                    is_streaming: false,
                                    is_complete: true,
                                    error: Some(format!("Request timeout after 30s. Model '{model_name}' may not be available.")),
                                    mcp_status: None,
                                };
                                let _ = tx_clone.send(timeout_response).await;
                            });
                        },
                        Err(e) => {
                            // If channel is full or closed, send error response back
                            let error_response = LlmResponse {
                                task_id: request.task_id,
                                model_name: request.model_name,
                                content: String::new(),
                                is_streaming: false,
                                is_complete: true,
                                error: Some(format!("Channel error: {e}")),
                                mcp_status: None,
                            };
                            let _ = new_llm_tx.send(error_response).await;
                        }
                    }
                }

                // Convert incoming String to LlmResponse
                Some(response) = llm_rx.recv() => {
                    // Get the current task from the queue
                    let (task_id, model_name) = if let Some(task_info) = request_queue.front() {
                        task_info.clone()
                    } else {
                        // No pending request, skip
                        continue;
                    };

                    // Parse the string-based protocol
                    let llm_response = if response == "<<STREAM_START>>" {
                        // Starting a new streaming response
                        LlmResponse {
                            task_id,
                            model_name,
                            content: String::new(),
                            is_streaming: true,
                            is_complete: false,
                            error: None,
                            mcp_status: None,
                        }
                    } else if let Some(chunk) = response.strip_prefix("<<STREAM_CHUNK>>") {
                        LlmResponse {
                            task_id,
                            model_name,
                            content: chunk.to_string(),
                            is_streaming: true,
                            is_complete: false,
                            error: None,
                            mcp_status: None,
                        }
                    } else if response == "<<STREAM_END>>" {
                        // Remove from queue and pending when complete
                        request_queue.pop_front();

                        LlmResponse {
                            task_id,
                            model_name,
                            content: String::new(),
                            is_streaming: true,
                            is_complete: true,
                            error: None,
                            mcp_status: None,
                        }
                    } else if let Some(error_msg) = response.strip_prefix("<<STREAM_ERROR>>") {
                        // Remove from queue and pending on error
                        request_queue.pop_front();

                        LlmResponse {
                            task_id,
                            model_name,
                            content: String::new(),
                            is_streaming: false,
                            is_complete: true,
                            error: Some(error_msg.to_string()),
                            mcp_status: None,
                        }
                    } else {
                        // Regular complete response
                        request_queue.pop_front();

                        LlmResponse {
                            task_id,
                            model_name,
                            content: response,
                            is_streaming: false,
                            is_complete: true,
                            error: None,
                            mcp_status: None,
                        }
                    };

                    let _ = new_llm_tx.send(llm_response).await;
                }

                else => break,
            }
        }
    });

    // Run with the adapted channels
    run_with_channels(new_ui_tx, new_llm_rx).await
}

#[cfg(any(feature = "llm-remote", feature = "llama-cpp"))]
fn extract_tool_calls(response: &str) -> Vec<(String, String)> {
    let mut tool_calls = Vec::new();

    // Remove markdown code blocks to find tools within them
    let cleaned_response = response.replace("```", "").replace('`', "");

    let mut remaining = &cleaned_response[..];

    while let Some(at_pos) = remaining.find('@') {
        // Check if this is a tool call (followed by word characters)
        let after_at = &remaining[at_pos + 1..];

        if let Some(space_or_brace) = after_at.find(|c: char| c.is_whitespace() || c == '{') {
            let tool_name = &after_at[..space_or_brace];

            // Only process if tool_name is alphanumeric and not "screenshot"
            if tool_name.chars().all(|c| c.is_alphanumeric() || c == '_')
                && tool_name != "screenshot"
            {
                let args_start = at_pos + 1 + space_or_brace;
                let args_str = &remaining[args_start..].trim_start();

                let (args, consumed_len) = if args_str.starts_with('{') {
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
                        (&args_str[..end_pos], end_pos)
                    } else {
                        // No closing brace found
                        ("", 0)
                    }
                } else {
                    // Take until newline or end of string
                    let end_pos = args_str.find('\n').unwrap_or(args_str.len());
                    (args_str[..end_pos].trim(), end_pos)
                };

                if !args.is_empty() {
                    tool_calls.push((tool_name.to_string(), args.to_string()));
                }

                // Move to after this tool call
                remaining = &remaining[args_start + consumed_len..];
            } else {
                // Not a valid tool call, move past the @
                remaining = &remaining[at_pos + 1..];
            }
        } else {
            // No space or brace after @, move past it
            remaining = &remaining[at_pos + 1..];
        }
    }

    tool_calls
}

pub async fn run_task_view(task_id: &str, session_id: &str) -> Result<()> {
    use crate::app::App;
    use tokio::sync::mpsc;

    // Create a task-specific app instance
    let mut app = App::new();

    // Set up IPC connection to main process
    let (_tx, mut rx) = mpsc::channel::<TaskViewMessage>(100);

    // Connect to main process via IPC
    let _ipc_handle = tokio::spawn(async move {
        // In production, this would connect via Unix socket or named pipe
        // For now, we'll use environment variables to get the connection info
        if let Ok(ipc_path) = std::env::var("ARKAVO_IPC_PATH") {
            // Connect to IPC endpoint
            println!("Connecting to IPC at: {ipc_path}");
        }
    });

    // Configure the app for task-specific view
    // These methods would be added to App in a full implementation
    // app.set_title(&format!("Task: {} (Session: {})", task_id, session_id));
    // app.set_read_only(false); // Allow interaction in task view
    println!("Running task view for task: {task_id} in session: {session_id}");

    // Handle task-specific events
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                TaskViewMessage::UpdateStatus(status) => {
                    println!("Task status updated: {status}");
                }
                TaskViewMessage::AppendOutput(output) => {
                    println!("Task output: {output}");
                }
                TaskViewMessage::Complete(result) => {
                    println!("Task completed: {result:?}");
                    break;
                }
            }
        }
    });

    // Run the terminal UI
    app.run().await
}

/// Messages for task-specific view communication
#[allow(dead_code)]
#[derive(Debug, Clone)]
enum TaskViewMessage {
    UpdateStatus(String),
    AppendOutput(String),
    Complete(TaskResult),
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct TaskResult {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct TerminalConfig {
    pub frame_budget_ms: u64,
    pub enable_mouse: bool,
    pub enable_alternate_screen: bool,
    pub max_fps: u32,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            frame_budget_ms: 8, // Target <8ms render time for 120fps
            enable_mouse: true,
            enable_alternate_screen: true,
            max_fps: 120,
        }
    }
}
