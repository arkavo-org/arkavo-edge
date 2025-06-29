pub mod app;
pub mod benchmark;
pub mod event;
pub mod helix;
pub mod multi_terminal;
pub mod renderer;
pub mod telemetry;
pub mod ui;
pub mod vim;

#[cfg(test)]
mod tests;

use anyhow::Result;
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
}

pub struct TerminalContext {
    pub message_tx: mpsc::Sender<ChatMessage>,
    pub message_rx: mpsc::Receiver<ChatMessage>,
}

pub async fn run() -> Result<()> {
    // Create channels for LLM communication
    let (ui_tx, mut ui_rx) = mpsc::channel::<LlmRequest>(100);
    let (llm_tx, llm_rx) = mpsc::channel::<LlmResponse>(100);

    // Spawn LLM handler task with proper Ollama integration
    tokio::spawn(async move {
        use arkavo_llm::Message;
        use tokio_stream::StreamExt;

        // Initialize LLM client using the same logic as chat command
        let client = match initialize_llm_client().await {
            Ok(client) => std::sync::Arc::new(client),
            Err(e) => {
                eprintln!("Failed to initialize LLM client: {e}");
                return;
            }
        };

        eprintln!(
            "Terminal UI connected to LLM provider: {}",
            client.provider_name()
        );

        // Keep conversation context
        let mut messages = Vec::new();

        while let Some(request) = ui_rx.recv().await {
            let llm_tx = llm_tx.clone();
            let mut messages_clone = messages.clone();

            // Add user message to context
            let user_message = Message::user(&request.prompt);
            messages_clone.push(user_message.clone());

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
                        if let Ok(all_configs) = storage.search("http", 20, Some("config")).await {
                            // Filter for Ollama server configs only
                            let ollama_configs: Vec<_> = all_configs
                                .into_iter()
                                .filter(|config| {
                                    config
                                        .memory
                                        .metadata
                                        .as_ref()
                                        .and_then(|m| m.get("type"))
                                        .and_then(|t| t.as_str())
                                        .map(|t| t == "arkavo_ollama_server_config")
                                        .unwrap_or(false)
                                })
                                .filter(|config| config.memory.content != "http://localhost:11434")
                                .collect();

                            // Extract server number from prefix (e.g., "server1" -> 1)
                            if let Some(num_str) = server_prefix.strip_prefix("server") {
                                if let Ok(idx) = num_str.parse::<usize>() {
                                    if idx > 0 && idx <= ollama_configs.len() {
                                        ollama_configs[idx - 1].memory.content.clone()
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

            // Create a new Ollama client with the specific model and server
            let model_specific_client = arkavo_llm::LlmClient::new(Box::new(
                arkavo_llm::ollama::OllamaClient::new(Some(server_url), Some(actual_model)),
            ));

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
                            })
                            .await;

                        while let Some(chunk_result) = stream.next().await {
                            match chunk_result {
                                Ok(chunk) => {
                                    if !chunk.content.is_empty() {
                                        // Send each chunk as it arrives
                                        let _ = llm_tx
                                            .send(LlmResponse {
                                                task_id: request.task_id,
                                                model_name: request.model_name.clone(),
                                                content: chunk.content,
                                                is_streaming: true,
                                                is_complete: false,
                                                error: None,
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
                                        })
                                        .await;
                                    break;
                                }
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
                            })
                            .await;
                    }
                    Err(e) => {
                        let _ = llm_tx
                            .send(LlmResponse {
                                task_id: request.task_id,
                                model_name: request.model_name,
                                content: String::new(),
                                is_streaming: false,
                                is_complete: true,
                                error: Some(format!("Failed to get LLM response: {e}")),
                            })
                            .await;
                    }
                }
            });

            // Update context
            messages.push(user_message);
        }
    });

    run_with_channels(ui_tx, llm_rx).await
}

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
                eprintln!("✓ Connected to saved Ollama server at {server_url}");
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
                    "Failed to connect to Ollama at {}: {}",
                    base_url,
                    e
                )),
            }
        }
        Err(e) => Err(anyhow::anyhow!("Failed to create client: {}", e)),
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

                    // For compatibility, just send the prompt (without model prefix for now)
                    // TODO: The architecture needs to be updated to support dynamic model selection
                    match ui_tx.try_send(request.prompt.clone()) {
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
                        }
                    } else if let Some(chunk) = response.strip_prefix("<<STREAM_CHUNK>>") {
                        LlmResponse {
                            task_id,
                            model_name,
                            content: chunk.to_string(),
                            is_streaming: true,
                            is_complete: false,
                            error: None,
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

pub async fn run_task_view(task_id: &str, session_id: &str) -> Result<()> {
    // TODO: Implement task-specific view that connects to main process
    let mut app = App::new();
    println!("Running task view for task: {task_id} in session: {session_id}");
    app.run().await
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
