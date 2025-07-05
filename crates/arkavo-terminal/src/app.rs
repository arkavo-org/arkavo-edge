use anyhow::Result;
use arkavo_test::mcp::mcp_connection::McpConnection;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    terminal,
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
};
use std::collections::HashMap;
use std::io;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::event::{AppEvent, EventHandler};
use crate::helix::HelixEditor;
use crate::renderer::Renderable;
use crate::telemetry::UITelemetry;
use crate::ui::{
    TaskManager, chat::ChatView, code::CodeView, dataflow::DataflowView, debug::DebugView,
    diff::DiffView,
};
use crate::vim::VimState;
use crate::{LlmRequest, LlmResponse};

pub enum ViewMode {
    Chat,
    Code,
    Diff,
    Debug,
    Dataflow,
}

#[derive(Clone, Copy, PartialEq)]
pub enum LayoutMode {
    Tabbed,   // Original tab-based layout
    Portrait, // Three-pane stacked layout for portrait monitors
}

pub enum FocusedPane {
    Diff,
    Thinking,
    Chat,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfigurationMode {
    None,
    OllamaServer { input: String, testing: bool },
    OpenAIKey { input: String },
    AnthropicKey { input: String },
}

#[derive(Debug, Clone)]
pub struct ProviderInfo {
    pub name: String,
    pub provider_type: ProviderType,
    pub url: Option<String>,
    pub status: ProviderStatus,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderType {
    Ollama,
    OpenAI,
    Anthropic,
    Claude,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderStatus {
    Connected,
    Disconnected,
    NotConfigured,
    Error(String),
}

pub struct App {
    pub should_quit: bool,
    pub view_mode: ViewMode,
    pub layout_mode: LayoutMode,
    pub focused_pane: FocusedPane,
    pub chat_view: ChatView,
    pub code_view: CodeView,
    pub diff_view: DiffView,
    pub debug_view: DebugView,
    pub dataflow_view: DataflowView,
    pub thinking_view: ChatView, // Reuse ChatView for chain-of-thought display
    pub event_handler: EventHandler,
    pub ui_tx: Option<mpsc::Sender<LlmRequest>>,
    pub llm_rx: Option<mpsc::Receiver<LlmResponse>>,
    pub thinking_rx: Option<mpsc::Receiver<String>>, // For chain-of-thought updates
    pub telemetry: UITelemetry,
    pub last_frame_time: Instant,
    pub frame_times: Vec<Duration>,
    pub vim_state: VimState,
    pub vim_enabled: bool,
    pub helix_editor: Option<HelixEditor>,
    pub task_manager: TaskManager,
    pub input_buffer: String,
    pub available_models: Vec<String>,
    pub selected_model: usize,
    pub input_focused: bool,
    pub last_quit_attempt: Option<Instant>,
    pub active_model: Option<String>, // The actual model being used
    pub main_task_id: Option<uuid::Uuid>, // ID of the main conversation task
    pub server_urls: HashMap<String, String>, // Map server names to URLs
    pub configuration_mode: ConfigurationMode, // Configuration dialog state
    pub providers: Vec<ProviderInfo>, // All available providers and their status
    pub mcp_client: Option<McpConnection>, // MCP client for tools
    pub mcp_tools: Vec<String>,       // Available MCP tool names
}

impl App {
    pub fn new() -> Self {
        let mut thinking_view = ChatView::new();
        thinking_view.add_message(
            crate::ui::chat::MessageRole::System,
            "Agent thinking will appear here...".to_string(),
        );

        Self {
            should_quit: false,
            view_mode: ViewMode::Chat,
            layout_mode: LayoutMode::Tabbed,
            focused_pane: FocusedPane::Chat,
            chat_view: ChatView::new(),
            code_view: CodeView::new(),
            diff_view: DiffView::new(),
            debug_view: DebugView::new(),
            dataflow_view: DataflowView::new(),
            thinking_view,
            event_handler: EventHandler::new(),
            ui_tx: None,
            llm_rx: None,
            thinking_rx: None,
            telemetry: UITelemetry::new(),
            last_frame_time: Instant::now(),
            frame_times: Vec::with_capacity(120), // Track last 120 frames (1 second at 120fps)
            vim_state: VimState::new(),
            vim_enabled: false, // Default to disabled for now
            helix_editor: HelixEditor::new().ok(),
            task_manager: TaskManager::new(),
            input_buffer: String::new(),
            available_models: vec![], // Models will be fetched dynamically
            selected_model: 0,
            input_focused: true,
            last_quit_attempt: None,
            active_model: None, // Will be set when models are fetched
            main_task_id: None,
            server_urls: HashMap::new(),
            configuration_mode: ConfigurationMode::None,
            providers: vec![
                ProviderInfo {
                    name: "OpenAI".to_string(),
                    provider_type: ProviderType::OpenAI,
                    url: Some("https://api.openai.com".to_string()),
                    status: ProviderStatus::NotConfigured,
                    models: vec![],
                },
                ProviderInfo {
                    name: "Anthropic".to_string(),
                    provider_type: ProviderType::Anthropic,
                    url: Some("https://api.anthropic.com".to_string()),
                    status: ProviderStatus::NotConfigured,
                    models: vec![],
                },
                ProviderInfo {
                    name: "MCP Tools".to_string(),
                    provider_type: ProviderType::Claude,
                    url: None,
                    status: ProviderStatus::Disconnected,
                    models: vec![],
                },
            ],
            mcp_client: None,
            mcp_tools: vec![],
        }
    }

    pub fn new_with_channels(
        ui_tx: mpsc::Sender<LlmRequest>,
        llm_rx: mpsc::Receiver<LlmResponse>,
    ) -> Self {
        let mut thinking_view = ChatView::new();
        thinking_view.add_message(
            crate::ui::chat::MessageRole::System,
            "Agent thinking will appear here...".to_string(),
        );

        Self {
            should_quit: false,
            view_mode: ViewMode::Chat,
            layout_mode: LayoutMode::Tabbed,
            focused_pane: FocusedPane::Chat,
            chat_view: ChatView::new(),
            code_view: CodeView::new(),
            diff_view: DiffView::new(),
            debug_view: DebugView::new(),
            dataflow_view: DataflowView::new(),
            thinking_view,
            event_handler: EventHandler::new(),
            ui_tx: Some(ui_tx),
            llm_rx: Some(llm_rx),
            thinking_rx: None,
            telemetry: UITelemetry::new(),
            last_frame_time: Instant::now(),
            frame_times: Vec::with_capacity(120),
            vim_state: VimState::new(),
            vim_enabled: false, // Default to disabled for now
            helix_editor: HelixEditor::new().ok(),
            task_manager: TaskManager::new(),
            input_buffer: String::new(),
            available_models: vec![], // Models will be fetched dynamically
            selected_model: 0,
            input_focused: true,
            last_quit_attempt: None,
            active_model: None, // Will be set when models are fetched
            main_task_id: None,
            server_urls: HashMap::new(),
            configuration_mode: ConfigurationMode::None,
            providers: vec![
                ProviderInfo {
                    name: "OpenAI".to_string(),
                    provider_type: ProviderType::OpenAI,
                    url: Some("https://api.openai.com".to_string()),
                    status: ProviderStatus::NotConfigured,
                    models: vec![],
                },
                ProviderInfo {
                    name: "Anthropic".to_string(),
                    provider_type: ProviderType::Anthropic,
                    url: Some("https://api.anthropic.com".to_string()),
                    status: ProviderStatus::NotConfigured,
                    models: vec![],
                },
                ProviderInfo {
                    name: "MCP Tools".to_string(),
                    provider_type: ProviderType::Claude,
                    url: None,
                    status: ProviderStatus::Disconnected,
                    models: vec![],
                },
            ],
            mcp_client: None,
            mcp_tools: vec![],
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;

        // Setup terminal
        self.setup_terminal()?;

        // Initialize MCP connection
        self.initialize_mcp_connection();

        // Ollama configuration is handled by arkavo chat command

        // Run the app
        let result = self.run_app(&mut terminal).await;

        // Always restore terminal state
        self.restore_terminal()?;

        result
    }

    fn setup_terminal(&self) -> Result<()> {
        terminal::enable_raw_mode()?;
        crossterm::execute!(
            io::stdout(),
            terminal::EnterAlternateScreen,
            event::EnableMouseCapture
        )?;
        Ok(())
    }

    fn restore_terminal(&self) -> Result<()> {
        // Ignore errors during cleanup to ensure we try all steps
        let _ = terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            io::stdout(),
            terminal::LeaveAlternateScreen,
            event::DisableMouseCapture
        );
        Ok(())
    }

    pub fn get_server_url_for_model(&self, model_name: &str) -> String {
        if let Some((server_prefix, _)) = model_name.split_once('/') {
            self.server_urls
                .get(server_prefix)
                .cloned()
                .unwrap_or_else(|| "http://localhost:11434".to_string())
        } else {
            "http://localhost:11434".to_string()
        }
    }

    pub fn initialize_mcp_connection(&mut self) {
        // Initialize MCP client - attempt by default unless explicitly disabled
        if std::env::var("ARKAVO_MCP_DISABLED").unwrap_or_default() == "true" {
            self.add_debug_log(
                crate::ui::debug::LogLevel::Info,
                "[MCP] MCP disabled by environment variable".to_string(),
            );
            return;
        }

        let result = McpConnection::new_in_process();

        match result {
            Ok(client) => {
                // List available tools
                let tools = client.list_tools();
                self.mcp_tools.clone_from(&tools);
                self.add_debug_log(
                    crate::ui::debug::LogLevel::Info,
                    format!("[MCP] Connected with {} tools available", tools.len()),
                );

                // Update MCP provider info
                if let Some(provider) = self.providers.iter_mut().find(|p| p.name == "MCP Tools") {
                    provider.status = ProviderStatus::Connected;
                    provider.models.clone_from(&tools);
                }
                self.mcp_client = Some(client);
            }
            Err(e) => {
                self.add_debug_log(
                    crate::ui::debug::LogLevel::Warning,
                    format!("[MCP] Failed to connect: {e}"),
                );

                // Update MCP provider status
                if let Some(provider) = self.providers.iter_mut().find(|p| p.name == "MCP Tools") {
                    provider.status = ProviderStatus::Error(format!("Failed to connect: {e}"));
                    provider.models = vec![];
                }
            }
        }
    }

    #[cfg(feature = "llm")]
    pub async fn fetch_available_models(&mut self) {
        // Try to fetch models from discovered Ollama servers
        use arkavo_llm::ollama::OllamaClient;
        use arkavo_memory::storage::MemoryStorage;
        use std::sync::Arc;

        let mut all_models = Vec::new();

        // Clear existing Ollama providers but keep frontier providers
        self.providers
            .retain(|p| p.provider_type != ProviderType::Ollama);

        // Initialize memory storage to check for saved Ollama configurations
        let storage = match MemoryStorage::new().await {
            Ok(s) => Arc::new(s),
            Err(e) => {
                self.add_debug_log(
                    crate::ui::debug::LogLevel::Error,
                    format!("[Storage] Failed to initialize: {e}"),
                );
                return;
            }
        };

        // Search for saved Ollama server configurations
        // Search directly for the type marker like the chat command does
        let saved_configs = match storage
            .search("arkavo_ollama_server_config", 20, Some("config"))
            .await
        {
            Ok(configs) => {
                self.add_debug_log(
                    crate::ui::debug::LogLevel::Debug,
                    format!("[Storage] Found {} saved Ollama configs", configs.len()),
                );
                configs
            }
            Err(e) => {
                self.add_debug_log(
                    crate::ui::debug::LogLevel::Error,
                    format!("[Storage] Failed to search configs: {e}"),
                );
                vec![]
            }
        };

        // Always try localhost first
        let localhost_client = OllamaClient::new(Some("http://localhost:11434".to_string()), None);
        match localhost_client.list_models().await {
            Ok(models) => {
                self.server_urls.insert(
                    "localhost".to_string(),
                    "http://localhost:11434".to_string(),
                );
                let model_count = models.len();
                let mut provider_models = Vec::new();
                for model in models {
                    let full_name = format!("localhost/{model}");
                    all_models.push(full_name.clone());
                    provider_models.push(model);
                }

                // Add localhost provider info
                self.providers.push(ProviderInfo {
                    name: "Ollama (localhost)".to_string(),
                    provider_type: ProviderType::Ollama,
                    url: Some("http://localhost:11434".to_string()),
                    status: ProviderStatus::Connected,
                    models: provider_models,
                });

                self.add_debug_log(
                    crate::ui::debug::LogLevel::Info,
                    format!("[Ollama] Connected to localhost, found {model_count} models"),
                );
            }
            Err(e) => {
                // Add disconnected localhost provider
                self.providers.push(ProviderInfo {
                    name: "Ollama (localhost)".to_string(),
                    provider_type: ProviderType::Ollama,
                    url: Some("http://localhost:11434".to_string()),
                    status: ProviderStatus::Disconnected,
                    models: vec![],
                });

                self.add_debug_log(
                    crate::ui::debug::LogLevel::Debug,
                    format!("[Ollama] Localhost not available: {e}"),
                );
            }
        }

        // Try saved configurations - filter for Ollama server configs
        // Sort configs by URL to ensure consistent server numbering
        let mut sorted_configs: Vec<_> = saved_configs
            .iter()
            .filter(|c| {
                let url = &c.memory.content;
                url != "CLEARED" && url != "http://localhost:11434" && url.starts_with("http")
            })
            .collect();
        sorted_configs.sort_by(|a, b| a.memory.content.cmp(&b.memory.content));

        let mut server_idx = 1;
        self.add_debug_log(
            crate::ui::debug::LogLevel::Debug,
            format!("[Models] Checking {} saved configs", sorted_configs.len()),
        );
        for config in sorted_configs.iter() {
            let server_url = &config.memory.content;

            self.add_debug_log(
                crate::ui::debug::LogLevel::Debug,
                format!("[Storage] Found saved Ollama config: {server_url}"),
            );

            if server_url.starts_with("http") {
                let client = OllamaClient::new(Some(server_url.clone()), None);

                match client.list_models().await {
                    Ok(models) => {
                        let server_name = format!("server{server_idx}");
                        self.server_urls
                            .insert(server_name.clone(), server_url.clone());

                        let model_count = models.len();
                        let mut provider_models = Vec::new();
                        for model in models {
                            let full_name = format!("{server_name}/{model}");
                            all_models.push(full_name);
                            provider_models.push(model);
                        }

                        // Add remote Ollama provider info
                        // Extract just the host:port from the URL for cleaner display
                        let display_url = server_url
                            .strip_prefix("http://")
                            .or_else(|| server_url.strip_prefix("https://"))
                            .unwrap_or(&server_url);
                        self.providers.push(ProviderInfo {
                            name: format!("Ollama ({server_name} - {display_url})"),
                            provider_type: ProviderType::Ollama,
                            url: Some(server_url.clone()),
                            status: ProviderStatus::Connected,
                            models: provider_models,
                        });

                        self.add_debug_log(
                            crate::ui::debug::LogLevel::Info,
                            format!("[Ollama] Connected to {server_name}: {server_url}, found {model_count} models"),
                        );
                        server_idx += 1;
                    }
                    Err(e) => {
                        // Add disconnected remote Ollama provider
                        let server_name = format!("server{server_idx}");
                        let display_url = server_url
                            .strip_prefix("http://")
                            .or_else(|| server_url.strip_prefix("https://"))
                            .unwrap_or(&server_url);
                        self.providers.push(ProviderInfo {
                            name: format!("Ollama ({server_name} - {display_url})"),
                            provider_type: ProviderType::Ollama,
                            url: Some(server_url.clone()),
                            status: ProviderStatus::Error(e.to_string()),
                            models: vec![],
                        });

                        self.add_debug_log(
                            crate::ui::debug::LogLevel::Error,
                            format!("[Ollama] Failed to connect to {server_url}: {e}"),
                        );
                        server_idx += 1;
                    }
                }
            }
        }

        if !all_models.is_empty() {
            self.available_models.clone_from(&all_models);
            // If we have models and none selected, select the first one
            if self.selected_model >= self.available_models.len() {
                self.selected_model = 0;
            }
            if self.active_model.is_none() && !self.available_models.is_empty() {
                self.active_model = Some(self.available_models[0].clone());
            }
            self.add_debug_log(
                crate::ui::debug::LogLevel::Info,
                format!(
                    "[Models] Found {} models: {:?}",
                    all_models.len(),
                    all_models
                ),
            );
        } else {
            // No models found, prompt for configuration
            self.available_models = vec![
                "Configure Ollama Server...".to_string(),
                "Configure OpenAI API...".to_string(),
                "Configure Anthropic API...".to_string(),
            ];
            self.selected_model = 0;
            self.add_debug_log(
                crate::ui::debug::LogLevel::Warning,
                "[Models] No models found, showing configuration options".to_string(),
            );
        }
    }

    #[cfg(not(feature = "llm"))]
    pub async fn fetch_available_models(&mut self) {
        // When LLM feature is disabled, show only configuration message
        self.available_models = vec!["LLM features disabled in this build".to_string()];
        self.selected_model = 0;
        self.add_debug_log(
            crate::ui::debug::LogLevel::Warning,
            "[Models] LLM features disabled in this build".to_string(),
        );
    }

    async fn run_app<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<()> {
        // Fetch available models from Ollama server
        self.fetch_available_models().await;

        // Immediate first draw for fast startup
        terminal.draw(|f| self.render(f))?;

        // Don't auto-launch Helix to prevent blank screen on startup
        // User can press 'e' when ready

        loop {
            // Check for LLM responses - drain all available messages
            let mut responses_to_process = Vec::new();
            let mut has_llm_updates = false;
            if let Some(ref mut llm_rx) = self.llm_rx {
                // Collect all available responses first
                while let Ok(response) = llm_rx.try_recv() {
                    responses_to_process.push(response);
                    has_llm_updates = true;
                }
            }

            // Process collected responses
            for response in responses_to_process {
                // Handle MCP status updates
                if let Some(mcp_status) = response.mcp_status {
                    if mcp_status.available {
                        self.add_debug_log(
                            crate::ui::debug::LogLevel::Info,
                            format!(
                                "[MCP] Connected with {} tools available",
                                mcp_status.tool_count
                            ),
                        );
                        // Update MCP provider status
                        if let Some(provider) =
                            self.providers.iter_mut().find(|p| p.name == "MCP Tools")
                        {
                            provider.status = ProviderStatus::Connected;
                            provider.models = vec![format!("{} tools", mcp_status.tool_count)];
                        }
                    } else if let Some(error_msg) = mcp_status.error_message {
                        self.add_debug_log(
                            crate::ui::debug::LogLevel::Warning,
                            format!("[MCP] {error_msg}"),
                        );
                        // Update MCP provider status
                        if let Some(provider) =
                            self.providers.iter_mut().find(|p| p.name == "MCP Tools")
                        {
                            provider.status = ProviderStatus::Error(error_msg);
                        }
                    }
                    // Skip further processing for MCP status updates
                    continue;
                }

                // Only log in debug builds
                #[cfg(debug_assertions)]
                self.add_debug_log(
                    crate::ui::debug::LogLevel::Debug,
                    format!(
                        "[UI] Received response for task {} from {}",
                        response.task_id, response.model_name
                    ),
                );

                // Find the task by ID
                if let Some(task) = self.task_manager.find_task_by_id_mut(response.task_id) {
                    if let Some(error) = response.error {
                        // Handle error
                        task.set_status(crate::ui::task_window::TaskStatus::Error);
                        task.add_message(
                            crate::ui::task_window::MessageRole::System,
                            format!("Error: {error}"),
                        );
                        // Only log errors in debug builds
                        #[cfg(debug_assertions)]
                        self.add_debug_log(
                            crate::ui::debug::LogLevel::Error,
                            format!("[UI] LLM error for task {}: {}", response.task_id, error),
                        );
                    } else if response.is_streaming && !response.is_complete {
                        // Handle streaming chunk
                        task.set_status(crate::ui::task_window::TaskStatus::Streaming);

                        // Check if we need to start a new assistant message
                        if task.messages.back().is_none_or(|m| {
                            m.role != crate::ui::task_window::MessageRole::Assistant
                        }) {
                            task.add_message(
                                crate::ui::task_window::MessageRole::Assistant,
                                response.content,
                            );
                        } else {
                            // Append to existing assistant message
                            if let Some(last_msg) = task.messages.back_mut() {
                                last_msg.content.push_str(&response.content);
                            }
                        }
                    } else if response.is_complete {
                        // Handle complete response
                        task.set_status(crate::ui::task_window::TaskStatus::Complete);

                        if response.is_streaming {
                            // Streaming is complete, message should already exist
                            self.telemetry.track_message_received();
                        } else {
                            // Non-streaming complete response
                            task.add_message(
                                crate::ui::task_window::MessageRole::Assistant,
                                response.content,
                            );
                            self.telemetry.track_message_received();
                        }

                        // Only log in debug builds
                        #[cfg(debug_assertions)]
                        self.add_debug_log(
                            crate::ui::debug::LogLevel::Info,
                            format!("[UI] Completed response for task {}", response.task_id),
                        );
                    }
                } else {
                    self.add_debug_log(
                        crate::ui::debug::LogLevel::Warning,
                        format!(
                            "[UI] Received response for unknown task {}",
                            response.task_id
                        ),
                    );
                }
            }

            // Check for thinking/chain-of-thought updates
            if let Some(ref mut thinking_rx) = self.thinking_rx {
                while let Ok(thought) = thinking_rx.try_recv() {
                    self.thinking_view
                        .add_message(crate::ui::chat::MessageRole::System, thought);
                }
            }

            // Poll for events with longer timeout when idle to reduce CPU usage
            let poll_timeout = if has_llm_updates || self.input_focused {
                Duration::from_millis(16) // 60fps when active
            } else {
                Duration::from_millis(100) // 10fps when idle
            };

            let mut needs_redraw = has_llm_updates;

            if event::poll(poll_timeout)? {
                match event::read()? {
                    Event::Key(key) => {
                        self.telemetry.track_key_event();

                        // Handle configuration mode input
                        if self.configuration_mode != ConfigurationMode::None {
                            match key.code {
                                KeyCode::Esc => {
                                    self.configuration_mode = ConfigurationMode::None;
                                }
                                KeyCode::Enter => {
                                    self.handle_configuration_submit().await;
                                }
                                KeyCode::Char(c) => match &mut self.configuration_mode {
                                    ConfigurationMode::OllamaServer { input, .. }
                                    | ConfigurationMode::OpenAIKey { input }
                                    | ConfigurationMode::AnthropicKey { input } => {
                                        input.push(c);
                                    }
                                    _ => {}
                                },
                                KeyCode::Backspace => match &mut self.configuration_mode {
                                    ConfigurationMode::OllamaServer { input, .. }
                                    | ConfigurationMode::OpenAIKey { input }
                                    | ConfigurationMode::AnthropicKey { input } => {
                                        input.pop();
                                    }
                                    _ => {}
                                },
                                _ => {}
                            }
                            continue;
                        }

                        match key.code {
                            KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                self.should_quit = true;
                            }
                            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                self.view_mode = ViewMode::Debug;
                                needs_redraw = true;
                            }
                            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                // Ctrl+S: Save/export UI state to file
                                self.export_ui_state();
                            }
                            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                // Ctrl+R: Refresh model list
                                self.add_debug_log(
                                    crate::ui::debug::LogLevel::Info,
                                    "[UI] Refreshing model list...".to_string(),
                                );
                                self.fetch_available_models().await;
                            }
                            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::ALT) => {
                                self.vim_enabled = !self.vim_enabled;
                                needs_redraw = true;
                                self.add_debug_log(
                                    crate::ui::debug::LogLevel::Info,
                                    format!(
                                        "[UI] Vim mode {}",
                                        if self.vim_enabled {
                                            "enabled"
                                        } else {
                                            "disabled"
                                        }
                                    ),
                                );
                            }
                            KeyCode::Char('e')
                                if self.input_focused
                                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                // Ctrl+E launches Helix editor
                                self.launch_helix_editor().await;
                                // Force immediate redraw after Helix
                                terminal.draw(|f| self.render(f))?;
                            }
                            KeyCode::Tab => {
                                // Cycle through available models (only if we have models)
                                if !self.available_models.is_empty() {
                                    self.selected_model =
                                        (self.selected_model + 1) % self.available_models.len();
                                    self.active_model =
                                        Some(self.available_models[self.selected_model].clone());

                                    // Log the model change for debugging
                                    self.add_debug_log(
                                        crate::ui::debug::LogLevel::Info,
                                        format!(
                                            "[UI] Switched to model: {}",
                                            self.active_model.as_ref().unwrap()
                                        ),
                                    );

                                    // If we selected a configuration option, show a hint
                                    if self
                                        .active_model
                                        .as_ref()
                                        .map(|m| m.ends_with("..."))
                                        .unwrap_or(false)
                                    {
                                        self.add_debug_log(
                                            crate::ui::debug::LogLevel::Info,
                                            format!(
                                                "[UI] Press Enter to {}",
                                                self.active_model.as_ref().unwrap()
                                            ),
                                        );
                                    } else {
                                        // Create or update task window for non-configuration models
                                        let model_name =
                                            self.active_model.as_ref().unwrap().clone();

                                        // Validate the model exists on its server
                                        let model_valid = if let Some((server_prefix, _)) =
                                            model_name.split_once('/')
                                        {
                                            // Find the provider for this server
                                            self.providers.iter().any(|p| {
                                                p.name.contains(server_prefix)
                                                    && p.status == ProviderStatus::Connected
                                                    && p.models
                                                        .iter()
                                                        .any(|m| model_name.ends_with(m))
                                            })
                                        } else {
                                            true // Assume valid if no server prefix
                                        };

                                        if !model_valid {
                                            self.add_debug_log(
                                                crate::ui::debug::LogLevel::Warning,
                                                format!(
                                                    "[UI] Model {model_name} not available on its server"
                                                ),
                                            );
                                        }

                                        // Check if we already have a task for this model
                                        let existing_task = self
                                            .task_manager
                                            .tasks
                                            .iter()
                                            .position(|t| t.model_name == model_name);

                                        if let Some(task_idx) = existing_task {
                                            // Switch to existing task
                                            self.task_manager.active_task = Some(task_idx);
                                            self.main_task_id =
                                                Some(self.task_manager.tasks[task_idx].id);
                                        } else {
                                            // Create new task for this model
                                            let id = uuid::Uuid::new_v4();
                                            let task = self.task_manager.create_task(
                                                "LLM Conversation".to_string(),
                                                model_name.clone(),
                                            );
                                            task.id = id;
                                            self.main_task_id = Some(id);
                                            let new_idx = self.task_manager.tasks.len() - 1;
                                            self.task_manager.active_task = Some(new_idx);

                                            self.add_debug_log(
                                                crate::ui::debug::LogLevel::Info,
                                                format!(
                                                    "[UI] Created task window for model: {model_name}"
                                                ),
                                            );
                                        }
                                    }
                                    // Force UI redraw after model change
                                    needs_redraw = true;
                                } else {
                                    self.add_debug_log(
                                        crate::ui::debug::LogLevel::Warning,
                                        "[UI] No models available for Tab cycling".to_string(),
                                    );
                                }
                            }
                            KeyCode::BackTab => {
                                if matches!(self.layout_mode, LayoutMode::Portrait) {
                                    self.prev_pane();
                                }
                            }
                            KeyCode::Esc => {
                                // Jump back to chat input in portrait mode
                                if matches!(self.layout_mode, LayoutMode::Portrait) {
                                    self.focused_pane = FocusedPane::Chat;
                                }
                            }
                            KeyCode::Enter => {
                                if self.input_focused {
                                    // Send message to the single conversation window
                                    if !self.input_buffer.is_empty() {
                                        // Use the actual model being used (from the LLM client)
                                        let displayed_model =
                                            self.active_model.clone().unwrap_or_else(|| {
                                                // If no model selected, try to use first available
                                                self.available_models
                                                    .first()
                                                    .cloned()
                                                    .unwrap_or_else(|| "unknown".to_string())
                                            });

                                        // Check if the selected model is actually a configuration option
                                        if displayed_model.ends_with("...") {
                                            // Enter configuration mode
                                            self.configuration_mode = match displayed_model.as_str()
                                            {
                                                "Configure Ollama Server..." => {
                                                    ConfigurationMode::OllamaServer {
                                                        input: String::new(),
                                                        testing: false,
                                                    }
                                                }
                                                "Configure OpenAI API..." => {
                                                    ConfigurationMode::OpenAIKey {
                                                        input: String::new(),
                                                    }
                                                }
                                                "Configure Anthropic API..." => {
                                                    ConfigurationMode::AnthropicKey {
                                                        input: String::new(),
                                                    }
                                                }
                                                _ => ConfigurationMode::None,
                                            };
                                            self.input_buffer.clear();
                                            continue;
                                        }

                                        // Find or create task for current model
                                        let task_id = if let Some(id) = self.main_task_id {
                                            // Verify the task exists and matches the current model
                                            if let Some(task) =
                                                self.task_manager.find_task_by_id(id)
                                            {
                                                if task.model_name == displayed_model {
                                                    id
                                                } else {
                                                    // Model changed, find or create task for new model
                                                    let existing_task =
                                                        self.task_manager.tasks.iter().position(
                                                            |t| t.model_name == displayed_model,
                                                        );

                                                    if let Some(task_idx) = existing_task {
                                                        let task_id =
                                                            self.task_manager.tasks[task_idx].id;
                                                        self.task_manager.active_task =
                                                            Some(task_idx);
                                                        self.main_task_id = Some(task_id);
                                                        task_id
                                                    } else {
                                                        // Create new task
                                                        let id = uuid::Uuid::new_v4();
                                                        let task = self.task_manager.create_task(
                                                            "LLM Conversation".to_string(),
                                                            displayed_model.clone(),
                                                        );
                                                        task.id = id;
                                                        self.main_task_id = Some(id);
                                                        let new_idx =
                                                            self.task_manager.tasks.len() - 1;
                                                        self.task_manager.active_task =
                                                            Some(new_idx);
                                                        id
                                                    }
                                                }
                                            } else {
                                                // Task doesn't exist, create new one
                                                let id = uuid::Uuid::new_v4();
                                                let task = self.task_manager.create_task(
                                                    "LLM Conversation".to_string(),
                                                    displayed_model.clone(),
                                                );
                                                task.id = id;
                                                self.main_task_id = Some(id);
                                                let new_idx = self.task_manager.tasks.len() - 1;
                                                self.task_manager.active_task = Some(new_idx);
                                                id
                                            }
                                        } else {
                                            // No main task, create new one
                                            let id = uuid::Uuid::new_v4();
                                            let task = self.task_manager.create_task(
                                                "LLM Conversation".to_string(),
                                                displayed_model.clone(),
                                            );
                                            task.id = id;
                                            self.main_task_id = Some(id);
                                            let new_idx = self.task_manager.tasks.len() - 1;
                                            self.task_manager.active_task = Some(new_idx);
                                            id
                                        };

                                        // Check for natural language Ollama commands
                                        let input_lower = self.input_buffer.to_lowercase();
                                        if input_lower.contains("add ollama server")
                                            || input_lower.contains("connect to ollama")
                                        {
                                            // Extract server address from the message
                                            if let Some(server_url) =
                                                self.extract_server_url(&self.input_buffer)
                                            {
                                                #[cfg(feature = "llm")]
                                                self.add_ollama_server(server_url).await;
                                                #[cfg(not(feature = "llm"))]
                                                {
                                                    self.add_debug_log(
                                                        crate::ui::debug::LogLevel::Error,
                                                        "LLM features disabled in this build"
                                                            .to_string(),
                                                    );
                                                }
                                                self.input_buffer.clear();
                                                continue;
                                            }
                                        }

                                        // Add user message to the task
                                        if let Some(task) =
                                            self.task_manager.find_task_by_id_mut(task_id)
                                        {
                                            task.add_message(
                                                crate::ui::task_window::MessageRole::User,
                                                self.input_buffer.clone(),
                                            );
                                            task.set_status(
                                                crate::ui::task_window::TaskStatus::Processing,
                                            );
                                        }

                                        // Send to LLM if channel is available
                                        if let Some(ref ui_tx) = self.ui_tx {
                                            // Always send with the same model name for now
                                            // The actual model is determined by the chat command
                                            let request = LlmRequest {
                                                task_id,
                                                model_name: displayed_model.clone(),
                                                prompt: self.input_buffer.clone(),
                                            };

                                            match ui_tx.try_send(request) {
                                                Ok(_) => {
                                                    self.add_debug_log(
                                                        crate::ui::debug::LogLevel::Info,
                                                        format!(
                                                            "[UI] Sent request for task {task_id}"
                                                        ),
                                                    );
                                                }
                                                Err(e) => {
                                                    self.add_debug_log(
                                                        crate::ui::debug::LogLevel::Error,
                                                        format!("[UI] Failed to send to LLM: {e}"),
                                                    );
                                                    if let Some(task) = self
                                                        .task_manager
                                                        .find_task_by_id_mut(task_id)
                                                    {
                                                        task.set_status(crate::ui::task_window::TaskStatus::Error);
                                                        task.add_message(
                                                            crate::ui::task_window::MessageRole::System,
                                                            format!("Failed to send request: {e}"),
                                                        );
                                                    }
                                                }
                                            }
                                        } else {
                                            self.add_debug_log(
                                                crate::ui::debug::LogLevel::Warning,
                                                "[UI] No LLM channel available".to_string(),
                                            );
                                            if let Some(task) =
                                                self.task_manager.find_task_by_id_mut(task_id)
                                            {
                                                task.set_status(
                                                    crate::ui::task_window::TaskStatus::Error,
                                                );
                                                task.add_message(
                                                    crate::ui::task_window::MessageRole::System,
                                                    "No LLM connection available".to_string(),
                                                );
                                            }
                                        }

                                        self.input_buffer.clear();
                                        self.telemetry.track_message_sent();

                                        // Keep focus on input field for continuous input
                                        self.input_focused = true;
                                    }
                                }
                            }
                            // Arrow keys and hjkl for navigation
                            KeyCode::Left | KeyCode::Char('h') if !self.input_focused => {
                                self.task_manager.prev_task();
                            }
                            KeyCode::Right | KeyCode::Char('l') if !self.input_focused => {
                                self.task_manager.next_task();
                            }
                            KeyCode::Up | KeyCode::Char('k') if !self.input_focused => {
                                if let Some(task) = self.task_manager.get_active_task() {
                                    task.scroll_up();
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') if !self.input_focused => {
                                if let Some(task) = self.task_manager.get_active_task() {
                                    task.scroll_down();
                                }
                            }
                            KeyCode::PageUp if !self.input_focused => {
                                if let Some(task) = self.task_manager.get_active_task() {
                                    // Scroll up by 10 lines
                                    for _ in 0..10 {
                                        task.scroll_up();
                                    }
                                }
                            }
                            KeyCode::PageDown if !self.input_focused => {
                                if let Some(task) = self.task_manager.get_active_task() {
                                    // Scroll down by 10 lines
                                    for _ in 0..10 {
                                        task.scroll_down();
                                    }
                                }
                            }
                            KeyCode::Home if !self.input_focused => {
                                if let Some(task) = self.task_manager.get_active_task() {
                                    task.scroll_offset = 0;
                                }
                            }
                            KeyCode::End if !self.input_focused => {
                                if let Some(task) = self.task_manager.get_active_task() {
                                    // Scroll to bottom - set a large offset, render will clamp it
                                    task.scroll_offset = u16::MAX;
                                }
                            }
                            // Number keys for quick jump (1-9)
                            KeyCode::Char(c) if !self.input_focused && c.is_ascii_digit() => {
                                if let Some(digit) = c.to_digit(10) {
                                    let index = (digit as usize).saturating_sub(1);
                                    if index < self.task_manager.tasks.len() {
                                        self.task_manager.active_task = Some(index);
                                    }
                                }
                            }
                            KeyCode::Char(c) if self.input_focused => {
                                // Add character to input buffer only when focused
                                // Prevent buffer overflow by limiting input length
                                if self.input_buffer.len() < 500 {
                                    self.input_buffer.push(c);
                                    needs_redraw = true;
                                }
                            }
                            KeyCode::Backspace if self.input_focused => {
                                // Remove last character from input buffer
                                self.input_buffer.pop();
                                needs_redraw = true;
                            }
                            _ => {
                                let event = AppEvent::from_crossterm_event(Event::Key(key));
                                self.handle_event(event)?;
                            }
                        }
                    }
                    Event::Resize(_, _) => {
                        needs_redraw = true;
                    }
                    _ => {}
                }
            }

            if self.should_quit {
                break;
            }

            // Only render when there are updates or user input
            if needs_redraw || has_llm_updates {
                terminal.draw(|f| {
                    // Always clear the entire frame to prevent artifacts
                    f.render_widget(ratatui::widgets::Clear, f.area());
                    self.render(f);
                })?;
            }

            // Track frame timing
            let now = Instant::now();
            let frame_time = now.duration_since(self.last_frame_time);
            self.last_frame_time = now;

            // Keep only last 120 frames
            if self.frame_times.len() >= 120 {
                self.frame_times.remove(0);
            }
            self.frame_times.push(frame_time);
        }

        Ok(())
    }

    fn render(&self, frame: &mut ratatui::Frame) {
        // Always use the new task-based layout
        self.render_task_layout(frame);

        // Render configuration dialog if active
        if self.configuration_mode != ConfigurationMode::None {
            self.render_configuration_dialog(frame);
        }
    }

    fn render_task_layout(&self, frame: &mut ratatui::Frame) {
        use ratatui::style::{Color, Modifier, Style};
        use ratatui::widgets::{Block, Borders, Paragraph};

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Input area at top
                Constraint::Length(8), // Model/Provider selector (increased for more info)
                Constraint::Min(10),   // Task windows
                Constraint::Length(1), // Status bar
            ])
            .split(frame.area());

        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if self.input_focused {
                Color::Cyan
            } else {
                Color::DarkGray
            }));
        let input_inner = input_block.inner(chunks[0]);
        frame.render_widget(input_block, chunks[0]);

        // Calculate visible portion of input buffer for scrolling
        let visible_width = input_inner.width.saturating_sub(1) as usize;

        // Show prompt with input buffer
        let prompt = " > ";
        let full_text = format!("{}{}", prompt, self.input_buffer);

        // Calculate visible portion with horizontal scrolling
        let scroll_offset = full_text.len().saturating_sub(visible_width);
        let input_text = full_text.chars().skip(scroll_offset).collect::<String>();

        let input_paragraph = Paragraph::new(input_text).style(Style::default());
        frame.render_widget(input_paragraph, input_inner);

        // Handle cursor visibility and position
        if self.input_focused {
            // Calculate visible portion of input buffer (including "> " prompt)
            let prompt_len = 2; // "> " is 2 characters
            let visible_width = input_inner.width.saturating_sub(1) as usize;
            let full_len = prompt_len + self.input_buffer.len();

            // Calculate scroll offset to keep cursor visible
            let scroll_offset = full_len.saturating_sub(visible_width);

            // Calculate cursor position within visible area
            let cursor_pos = prompt_len + self.input_buffer.len();
            let cursor_offset = cursor_pos.saturating_sub(scroll_offset);
            let cursor_x = (input_inner.x + cursor_offset as u16)
                .min(input_inner.x + input_inner.width.saturating_sub(1));
            let cursor_y = input_inner.y;

            // Show the cursor
            let _ = crossterm::execute!(io::stdout(), cursor::Show);
            frame.set_cursor_position((cursor_x, cursor_y));
        } else {
            // Hide cursor when not focused on input
            let _ = crossterm::execute!(io::stdout(), cursor::Hide);
        }

        // Render model selector with indication of actual model
        let active_model_display = if let Some(ref active) = self.active_model {
            format!("Active Model: {active}")
        } else {
            "Active Model: None".to_string()
        };

        // Create a block for the provider/model area
        let model_selector_block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" Providers & Models | {active_model_display} "))
            .border_style(Style::default().fg(Color::Yellow));

        let model_inner = model_selector_block.inner(chunks[1]);
        frame.render_widget(model_selector_block, chunks[1]);

        // Render provider and model information
        use ratatui::text::{Line, Span};

        let mut lines = Vec::new();

        // Group providers by type
        let mut ollama_providers = Vec::new();
        let mut frontier_providers = Vec::new();
        let mut mcp_provider = None;

        for provider in &self.providers {
            if provider.name == "MCP Tools" {
                mcp_provider = Some(provider);
            } else {
                match provider.provider_type {
                    ProviderType::Ollama => ollama_providers.push(provider),
                    ProviderType::OpenAI | ProviderType::Anthropic | ProviderType::Claude => {
                        frontier_providers.push(provider);
                    }
                }
            }
        }

        // Display Ollama providers
        if !ollama_providers.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                "Ollama Servers",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]));

            for provider in ollama_providers {
                let status_symbol = match &provider.status {
                    ProviderStatus::Connected => "✓",
                    ProviderStatus::Disconnected => "✗",
                    ProviderStatus::NotConfigured => "○",
                    ProviderStatus::Error(_) => "!",
                };

                let status_color = match &provider.status {
                    ProviderStatus::Connected => Color::Green,
                    ProviderStatus::Disconnected => Color::Red,
                    ProviderStatus::NotConfigured => Color::DarkGray,
                    ProviderStatus::Error(_) => Color::Red,
                };

                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(status_symbol, Style::default().fg(status_color)),
                    Span::raw(" "),
                    Span::raw(&provider.name),
                    Span::raw(" "),
                    Span::styled(
                        format!("({} models)", provider.models.len()),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
        }

        // Display frontier providers
        if !frontier_providers.is_empty() {
            if !lines.is_empty() {
                lines.push(Line::from("")); // Add spacing
            }

            lines.push(Line::from(vec![Span::styled(
                "Frontier Providers",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )]));

            for provider in frontier_providers {
                let status_symbol = match &provider.status {
                    ProviderStatus::Connected => "✓",
                    ProviderStatus::Disconnected => "✗",
                    ProviderStatus::NotConfigured => "○",
                    ProviderStatus::Error(_) => "!",
                };

                let status_color = match &provider.status {
                    ProviderStatus::Connected => Color::Green,
                    ProviderStatus::Disconnected => Color::Red,
                    ProviderStatus::NotConfigured => Color::DarkGray,
                    ProviderStatus::Error(_) => Color::Red,
                };

                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(status_symbol, Style::default().fg(status_color)),
                    Span::raw(" "),
                    Span::raw(&provider.name),
                    Span::raw(" "),
                    Span::styled(
                        if provider.status == ProviderStatus::NotConfigured {
                            "(not configured)".to_string()
                        } else {
                            format!("({} models)", provider.models.len())
                        },
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
        }

        // Display MCP Tools section
        if let Some(provider) = mcp_provider {
            if !lines.is_empty() {
                lines.push(Line::from("")); // Add spacing
            }

            lines.push(Line::from(vec![Span::styled(
                "MCP Tools",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )]));

            let status_symbol = match &provider.status {
                ProviderStatus::Connected => "✓",
                ProviderStatus::Disconnected => "✗",
                _ => "○",
            };

            let status_color = match &provider.status {
                ProviderStatus::Connected => Color::Green,
                ProviderStatus::Disconnected => Color::Red,
                _ => Color::DarkGray,
            };

            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(status_symbol, Style::default().fg(status_color)),
                Span::raw(" "),
                Span::raw(&provider.name),
                Span::raw(" "),
                Span::styled(
                    if provider.status == ProviderStatus::Connected {
                        format!("({} tools available)", self.mcp_tools.len())
                    } else {
                        "(not connected)".to_string()
                    },
                    Style::default().fg(Color::DarkGray),
                ),
            ]));

            // Show first few tools if connected
            if provider.status == ProviderStatus::Connected && !self.mcp_tools.is_empty() {
                let tools_to_show = self.mcp_tools.iter().take(3);
                for tool in tools_to_show {
                    lines.push(Line::from(vec![
                        Span::raw("    • "),
                        Span::styled(tool, Style::default().fg(Color::Blue)),
                    ]));
                }
                if self.mcp_tools.len() > 3 {
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled(
                            format!("... and {} more", self.mcp_tools.len() - 3),
                            Style::default()
                                .fg(Color::DarkGray)
                                .add_modifier(Modifier::ITALIC),
                        ),
                    ]));
                }
            }
        }

        // Add available models section
        if !self.available_models.is_empty() {
            if !lines.is_empty() {
                lines.push(Line::from("")); // Add spacing
            }
            lines.push(Line::from(vec![Span::styled(
                "Available Models (Tab to cycle):",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )]));

            for (idx, model) in self.available_models.iter().enumerate() {
                let is_selected = idx == self.selected_model;
                let prefix = if is_selected { "→ " } else { "  " };

                let style = if is_selected {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                lines.push(Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(model, style),
                ]));
            }
        }

        // Add help text at bottom
        if lines.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                "No providers available. Press Enter to configure.",
                Style::default().fg(Color::DarkGray),
            )]));
        } else {
            lines.push(Line::from("")); // Add spacing
            lines.push(Line::from(vec![Span::styled(
                "Tab: cycle models | Enter: configure | Ctrl+T: show all MCP tools",
                Style::default().fg(Color::DarkGray),
            )]));
        }

        let paragraph = Paragraph::new(lines);
        frame.render_widget(paragraph, model_inner);

        // Render task windows
        if self.task_manager.tasks.is_empty() {
            let help_text = r#"
Welcome to Arkavo Terminal UI

• Type your prompt after > and press Enter to start a conversation
• Press Tab to cycle through available models
• Press Ctrl+R to refresh model list
• Press Ctrl+Q to quit

Natural Language Commands:
• "Add Ollama server 192.168.1.100:11434" - Connect to a new Ollama server
• "Connect to Ollama at server.local:11434" - Alternative syntax

Scrolling (when in scroll mode):
• Arrow Up/Down or j/k to scroll line by line
• Page Up/Down to scroll by pages
• Home/End to jump to top/bottom
            "#;

            let help_paragraph = Paragraph::new(help_text)
                .style(Style::default().fg(Color::DarkGray))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Getting Started "),
                );

            frame.render_widget(help_paragraph, chunks[2]);
        } else {
            // Render active tasks in a grid (cap at 9 visible)
            let task_count = self.task_manager.tasks.len();
            let visible_count = task_count.min(9);
            let cols = ((visible_count as f32).sqrt().ceil() as usize).clamp(1, 3);
            let rows = visible_count.div_ceil(cols).min(3);

            // If there are more than 9 tasks, show a pager
            if task_count > 9 {
                let pager_text =
                    format!(" Showing 1-9 of {task_count} tasks (use 1-9 keys to jump) ");
                let pager_area = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(0)])
                    .split(chunks[2])[0];

                let pager = Paragraph::new(pager_text)
                    .style(Style::default().fg(Color::Yellow))
                    .block(Block::default().borders(Borders::NONE));
                frame.render_widget(pager, pager_area);

                // Adjust area for task grid
                let task_area = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(0)])
                    .split(chunks[2])[1];

                let row_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints(vec![Constraint::Percentage((100 / rows) as u16); rows])
                    .split(task_area);

                let mut task_idx = 0;
                for row_chunk in row_chunks.iter() {
                    let col_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints(vec![Constraint::Percentage((100 / cols) as u16); cols])
                        .split(*row_chunk);

                    for col_chunk in col_chunks.iter() {
                        if task_idx < visible_count {
                            let is_active = self.task_manager.active_task == Some(task_idx);
                            self.task_manager.tasks[task_idx].render(frame, *col_chunk, is_active);
                            task_idx += 1;
                        }
                    }
                }
            } else {
                // Normal grid for 9 or fewer tasks
                let row_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints(vec![Constraint::Percentage((100 / rows) as u16); rows])
                    .split(chunks[2]);

                let mut task_idx = 0;
                for row_chunk in row_chunks.iter() {
                    let col_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints(vec![Constraint::Percentage((100 / cols) as u16); cols])
                        .split(*row_chunk);

                    for col_chunk in col_chunks.iter() {
                        if task_idx < self.task_manager.tasks.len() {
                            let is_active = self.task_manager.active_task == Some(task_idx);
                            self.task_manager.tasks[task_idx].render(frame, *col_chunk, is_active);
                            task_idx += 1;
                        }
                    }
                }
            }
        }

        // Render status bar
        self.render_status_bar(frame, chunks[3]);
    }

    #[allow(dead_code)]
    fn render_tabbed_layout(&mut self, frame: &mut ratatui::Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Status bar
                Constraint::Min(0),    // Main content
                Constraint::Length(1), // Performance metrics
            ])
            .split(frame.area());

        // Render status bar
        self.render_status_bar(frame, chunks[0]);

        match self.view_mode {
            ViewMode::Chat => self.chat_view.render(frame, chunks[1]),
            ViewMode::Code => self.code_view.render(frame, chunks[1]),
            ViewMode::Diff => self.diff_view.render(frame, chunks[1]),
            ViewMode::Debug => self.debug_view.render(frame, chunks[1]),
            ViewMode::Dataflow => self.dataflow_view.render(frame, chunks[1]),
        }

        // Render performance metrics
        self.render_performance_bar(frame, chunks[2]);
    }

    #[allow(dead_code)]
    fn render_portrait_layout(&mut self, frame: &mut ratatui::Frame) {
        use ratatui::style::{Color, Style};
        use ratatui::widgets::{Block, Borders};

        // Three-pane stacked layout for portrait monitors
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(38), // Diff/Code Review
                Constraint::Percentage(27), // Agent Thinking
                Constraint::Min(10),        // Chat (remaining space)
            ])
            .split(frame.area());

        // Helper to get border style based on focus
        let is_focused = |pane: &FocusedPane| -> bool {
            matches!(
                (&self.focused_pane, pane),
                (FocusedPane::Diff, FocusedPane::Diff)
                    | (FocusedPane::Thinking, FocusedPane::Thinking)
                    | (FocusedPane::Chat, FocusedPane::Chat)
            )
        };

        // Render diff/code in top pane with border
        let diff_block = Block::default()
            .borders(Borders::ALL)
            .title(" Diff / Code Review ")
            .border_style(if is_focused(&FocusedPane::Diff) {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            });
        let diff_area = diff_block.inner(chunks[0]);
        frame.render_widget(diff_block, chunks[0]);

        match self.view_mode {
            ViewMode::Diff => self.diff_view.render(frame, diff_area),
            _ => self.code_view.render(frame, diff_area),
        }

        // Render thinking/chain-of-thought in middle pane with border
        let thinking_block = Block::default()
            .borders(Borders::ALL)
            .title(" Agent Thinking ")
            .border_style(if is_focused(&FocusedPane::Thinking) {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            });
        let thinking_area = thinking_block.inner(chunks[1]);
        frame.render_widget(thinking_block, chunks[1]);
        self.thinking_view.render(frame, thinking_area);

        // Render chat in bottom pane with border
        let chat_block = Block::default()
            .borders(Borders::ALL)
            .title(" Chat ")
            .border_style(if is_focused(&FocusedPane::Chat) {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            });
        let chat_area = chat_block.inner(chunks[2]);
        frame.render_widget(chat_block, chunks[2]);
        self.chat_view.render(frame, chat_area);
    }

    #[allow(dead_code)]
    fn switch_view(&mut self) {
        self.view_mode = match self.view_mode {
            ViewMode::Chat => ViewMode::Code,
            ViewMode::Code => ViewMode::Diff,
            ViewMode::Diff => ViewMode::Debug,
            ViewMode::Debug => ViewMode::Dataflow,
            ViewMode::Dataflow => ViewMode::Chat,
        };
    }

    fn handle_event(&mut self, event: AppEvent) -> Result<()> {
        match self.layout_mode {
            LayoutMode::Tabbed => match self.view_mode {
                ViewMode::Chat => self.chat_view.handle_event(event),
                ViewMode::Code => self.code_view.handle_event(event),
                ViewMode::Diff => self.diff_view.handle_event(event),
                ViewMode::Debug => self.debug_view.handle_event(event),
                ViewMode::Dataflow => Ok(()), // Dataflow view doesn't handle events yet
            },
            LayoutMode::Portrait => match self.focused_pane {
                FocusedPane::Chat => self.chat_view.handle_event(event),
                FocusedPane::Thinking => self.thinking_view.handle_event(event),
                FocusedPane::Diff => match self.view_mode {
                    ViewMode::Diff => self.diff_view.handle_event(event),
                    _ => self.code_view.handle_event(event),
                },
            },
        }
    }

    #[allow(dead_code)]
    fn next_pane(&mut self) {
        self.focused_pane = match self.focused_pane {
            FocusedPane::Diff => FocusedPane::Thinking,
            FocusedPane::Thinking => FocusedPane::Chat,
            FocusedPane::Chat => FocusedPane::Diff,
        };
        self.telemetry.track_pane_focus_change();
    }

    fn prev_pane(&mut self) {
        self.focused_pane = match self.focused_pane {
            FocusedPane::Diff => FocusedPane::Chat,
            FocusedPane::Thinking => FocusedPane::Diff,
            FocusedPane::Chat => FocusedPane::Thinking,
        };
        self.telemetry.track_pane_focus_change();
    }

    fn render_status_bar(&self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        use ratatui::style::{Color, Style};
        use ratatui::widgets::{Block, Borders, Paragraph};

        let active_model = self
            .active_model
            .clone()
            .unwrap_or_else(|| "No model selected".to_string());

        let mode_indicator = if self.input_focused {
            "INPUT MODE"
        } else {
            "SCROLL MODE (↑↓/jk/PgUp/PgDn/Home/End)"
        };

        let mcp_status = if self.mcp_client.is_some() {
            format!(" | MCP: ✓ {} tools", self.mcp_tools.len())
        } else {
            " | MCP: ✗".to_string()
        };

        let status = format!(
            " {mode_indicator} | Model: {active_model}{mcp_status} | Ctrl+R: Refresh | Ctrl+Q: Quit "
        );

        let paragraph = Paragraph::new(status)
            .style(Style::default().fg(Color::White).bg(Color::DarkGray))
            .block(Block::default().borders(Borders::NONE));

        frame.render_widget(paragraph, area);
    }

    #[allow(dead_code)]
    fn render_performance_bar(&self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        use ratatui::style::{Color, Style};
        use ratatui::widgets::{Block, Borders, Paragraph};

        // Calculate average FPS from frame times
        let fps = if !self.frame_times.is_empty() {
            let avg_frame_time: Duration =
                self.frame_times.iter().sum::<Duration>() / self.frame_times.len() as u32;
            if avg_frame_time.as_micros() > 0 {
                1_000_000 / avg_frame_time.as_micros()
            } else {
                120
            }
        } else {
            0
        };

        // Get messages stats
        let messages_sent = self
            .telemetry
            .messages_sent
            .load(std::sync::atomic::Ordering::Relaxed);
        let messages_received = self
            .telemetry
            .messages_received
            .load(std::sync::atomic::Ordering::Relaxed);

        // Check if currently streaming
        let is_streaming = self
            .chat_view
            .messages
            .back()
            .is_some_and(|m| m.is_streaming);

        let streaming_indicator = if is_streaming { " ● Streaming" } else { "" };

        let perf_text = format!(
            " {fps} FPS | Sent: {messages_sent} | Received: {messages_received}{streaming_indicator}"
        );

        let color = if fps >= 100 {
            Color::Green
        } else if fps >= 60 {
            Color::Yellow
        } else {
            Color::Red
        };

        let paragraph = Paragraph::new(perf_text)
            .style(Style::default().fg(color))
            .block(Block::default().borders(Borders::NONE));

        frame.render_widget(paragraph, area);
    }

    async fn launch_helix_editor(&mut self) {
        if let Some(ref helix) = self.helix_editor {
            // Save terminal state
            let _ = crossterm::execute!(io::stdout(), cursor::SavePosition);

            // Suspend the terminal temporarily
            let _ = terminal::disable_raw_mode();
            let _ = crossterm::execute!(io::stdout(), terminal::LeaveAlternateScreen, cursor::Show);

            // Get current input buffer content
            let initial_content = self.input_buffer.clone();

            // Launch helix and get the edited content
            match helix.launch_with_content(&initial_content) {
                Ok(edited_content) => {
                    // Update the input buffer with edited content
                    self.input_buffer = edited_content.trim_end().to_string();
                    self.add_debug_log(
                        crate::ui::debug::LogLevel::Info,
                        format!(
                            "[UI] Helix editor: {} chars pasted",
                            self.input_buffer.len()
                        ),
                    );
                }
                Err(e) => {
                    self.add_debug_log(
                        crate::ui::debug::LogLevel::Error,
                        format!("[UI] Helix editor error: {e}"),
                    );
                }
            }

            // Restore terminal with proper cleanup
            let _ = terminal::enable_raw_mode();
            let _ = crossterm::execute!(
                io::stdout(),
                terminal::EnterAlternateScreen,
                terminal::Clear(terminal::ClearType::All),
                cursor::Hide,
                cursor::RestorePosition
            );

            // Add a small delay for terminal to stabilize
            tokio::time::sleep(Duration::from_millis(50)).await;

            // No need to manually clear - the next render cycle will handle it
        } else {
            self.add_debug_log(
                crate::ui::debug::LogLevel::Error,
                "[UI] Helix unavailable—install helix or check logs".to_string(),
            );
        }
    }

    pub fn add_debug_log(&mut self, level: crate::ui::debug::LogLevel, message: String) {
        self.debug_view.add_log(level, message);
    }

    fn extract_server_url(&self, input: &str) -> Option<String> {
        // Look for patterns like IP:port, hostname:port, or URLs
        let patterns = [
            // IP address with port
            r"\b(?:\d{1,3}\.){3}\d{1,3}:\d{1,5}\b",
            // Hostname with port
            r"\b[a-zA-Z0-9\-\.]+:\d{1,5}\b",
            // Full URL
            r"https?://[^\s]+",
        ];

        for pattern in patterns {
            if let Ok(re) = regex::Regex::new(pattern) {
                if let Some(mat) = re.find(input) {
                    return Some(mat.as_str().to_string());
                }
            }
        }
        None
    }

    #[cfg(feature = "llm")]
    async fn add_ollama_server(&mut self, server_url: String) {
        use arkavo_llm::ollama::OllamaClient;
        use arkavo_memory::storage::MemoryStorage;
        use std::sync::Arc;

        // Normalize URL
        let mut url = server_url.trim().to_string();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            url = format!("http://{url}");
        }

        self.add_debug_log(
            crate::ui::debug::LogLevel::Info,
            format!("[Config] Testing connection to {url}"),
        );

        // Test connection
        let client = OllamaClient::new(Some(url.clone()), None);
        match client.list_models().await {
            Ok(models) => {
                // Save configuration
                if let Ok(storage) = MemoryStorage::new().await {
                    let storage = Arc::new(storage);
                    let embedding = vec![0.0; 384]; // Placeholder

                    let memory = arkavo_memory::models::Memory {
                        id: uuid::Uuid::new_v4(),
                        content: url.clone(),
                        metadata: Some(serde_json::json!({
                            "type": "arkavo_ollama_server_config",
                            "timestamp": chrono::Utc::now().to_rfc3339(),
                            "model_count": models.len(),
                        })),
                        category: Some("config".to_string()),
                        embedding,
                        created_at: chrono::Utc::now(),
                        updated_at: chrono::Utc::now(),
                    };

                    if let Err(e) = storage.store(memory).await {
                        self.add_debug_log(
                            crate::ui::debug::LogLevel::Error,
                            format!("[Config] Failed to save: {e}"),
                        );
                    } else {
                        self.add_debug_log(
                            crate::ui::debug::LogLevel::Info,
                            format!(
                                "[Config] Added Ollama server {} with {} models",
                                url,
                                models.len()
                            ),
                        );

                        // Refresh model list
                        self.fetch_available_models().await;
                    }
                }
            }
            Err(e) => {
                self.add_debug_log(
                    crate::ui::debug::LogLevel::Error,
                    format!("[Config] Failed to connect to {url}: {e}"),
                );
            }
        }
    }

    fn export_ui_state(&mut self) {
        use chrono::Local;
        use serde_json::json;
        use std::fs;

        // Create a comprehensive UI state dump
        let ui_state = json!({
            "timestamp": Local::now().to_rfc3339(),
            "providers": self.providers.iter().map(|p| {
                json!({
                    "name": p.name,
                    "type": format!("{:?}", p.provider_type),
                    "url": p.url,
                    "status": format!("{:?}", p.status),
                    "models": p.models,
                })
            }).collect::<Vec<_>>(),
            "active_model": self.active_model,
            "available_models": self.available_models,
            "selected_model": self.selected_model,
            "server_urls": self.server_urls,
            "debug_logs": self.debug_view.get_all_logs(),
            "tasks": self.task_manager.tasks.iter().map(|t| {
                json!({
                    "id": t.id.to_string(),
                    "agent": t.agent_name,
                    "model": t.model_name,
                    "status": format!("{:?}", t.status),
                    "messages": t.messages.iter().map(|m| {
                        json!({
                            "role": format!("{:?}", m.role),
                            "content": m.content,
                        })
                    }).collect::<Vec<_>>(),
                })
            }).collect::<Vec<_>>(),
        });

        // Generate filename with timestamp
        let filename = format!(
            "arkavo-ui-state-{}.json",
            Local::now().format("%Y%m%d-%H%M%S")
        );

        match fs::write(
            &filename,
            serde_json::to_string_pretty(&ui_state).unwrap_or_default(),
        ) {
            Ok(_) => {
                self.add_debug_log(
                    crate::ui::debug::LogLevel::Info,
                    format!("[Export] UI state saved to {filename}"),
                );
            }
            Err(e) => {
                self.add_debug_log(
                    crate::ui::debug::LogLevel::Error,
                    format!("[Export] Failed to save UI state: {e}"),
                );
            }
        }
    }

    fn render_configuration_dialog(&self, frame: &mut ratatui::Frame) {
        use ratatui::style::{Color, Modifier, Style};
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

        // Create a centered modal dialog
        let area = frame.area();
        let dialog_width = 60.min(area.width - 4);
        let dialog_height = 12.min(area.height - 4);

        let x = (area.width.saturating_sub(dialog_width)) / 2;
        let y = (area.height.saturating_sub(dialog_height)) / 2;

        let dialog_area = ratatui::layout::Rect::new(x, y, dialog_width, dialog_height);

        // Clear the area behind the dialog
        frame.render_widget(Clear, dialog_area);

        // Determine dialog title and content based on configuration mode
        let (title, prompt, input_value, status) = match &self.configuration_mode {
            ConfigurationMode::OllamaServer { input, testing } => {
                let status = if *testing {
                    "Testing connection..."
                } else {
                    ""
                };
                (
                    " Configure Ollama Server ",
                    "Enter server address (e.g., 192.168.1.100:11434):",
                    input.as_str(),
                    status,
                )
            }
            ConfigurationMode::OpenAIKey { input } => (
                " Configure OpenAI API ",
                "Enter your OpenAI API key:",
                input.as_str(),
                "Note: OpenAI provider not yet implemented",
            ),
            ConfigurationMode::AnthropicKey { input } => (
                " Configure Anthropic API ",
                "Enter your Anthropic API key:",
                input.as_str(),
                "Note: Anthropic provider not yet implemented",
            ),
            ConfigurationMode::None => return,
        };

        // Create the dialog block
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);

        // Create dialog content
        let mut lines = vec![
            Line::from(""),
            Line::from(Span::styled(prompt, Style::default().fg(Color::White))),
            Line::from(""),
            Line::from(vec![
                Span::raw("> "),
                Span::styled(input_value, Style::default().fg(Color::Yellow)),
                Span::styled("_", Style::default().add_modifier(Modifier::SLOW_BLINK)),
            ]),
            Line::from(""),
        ];

        if !status.is_empty() {
            lines.push(Line::from(Span::styled(
                status,
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )));
            lines.push(Line::from(""));
        }

        lines.push(Line::from(Span::styled(
            "Press Enter to submit, Esc to cancel",
            Style::default().fg(Color::DarkGray),
        )));

        let paragraph = Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .alignment(ratatui::layout::Alignment::Left);

        frame.render_widget(paragraph, inner);
    }

    async fn handle_configuration_submit(&mut self) {
        use arkavo_memory::storage::MemoryStorage;

        // Clone the configuration mode to avoid borrow checker issues
        let config_mode = self.configuration_mode.clone();

        match config_mode {
            ConfigurationMode::OllamaServer { input, .. } => {
                if input.is_empty() {
                    self.configuration_mode = ConfigurationMode::None;
                    return;
                }

                let base_url = if input.starts_with("http://") || input.starts_with("https://") {
                    input.clone()
                } else {
                    format!("http://{input}")
                };

                // Set testing mode
                if let ConfigurationMode::OllamaServer { testing, .. } =
                    &mut self.configuration_mode
                {
                    *testing = true;
                }

                // Test the connection
                let test_client =
                    arkavo_llm::ollama::OllamaClient::new(Some(base_url.clone()), None);

                match test_client.list_models().await {
                    Ok(models) => {
                        self.add_debug_log(
                            crate::ui::debug::LogLevel::Info,
                            format!(
                                "[Config] Connected to {base_url}, found {} models",
                                models.len()
                            ),
                        );

                        // Save the configuration
                        if let Ok(storage) = MemoryStorage::new().await {
                            let embedding_service =
                                arkavo_memory::embeddings::EmbeddingService::new();
                            let embedding =
                                match embedding_service.generate_embedding(&base_url).await {
                                    Ok(e) => e,
                                    Err(_) => vec![0.0; 384],
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
                                self.add_debug_log(
                                    crate::ui::debug::LogLevel::Error,
                                    format!("[Config] Failed to save configuration: {e}"),
                                );
                            } else {
                                self.add_debug_log(
                                    crate::ui::debug::LogLevel::Info,
                                    "[Config] Configuration saved successfully".to_string(),
                                );
                            }
                        }

                        // Exit configuration mode
                        self.configuration_mode = ConfigurationMode::None;

                        // Refresh available models
                        self.fetch_available_models().await;
                    }
                    Err(e) => {
                        self.add_debug_log(
                            crate::ui::debug::LogLevel::Error,
                            format!("[Config] Failed to connect to {base_url}: {e}"),
                        );
                        // Reset testing state but stay in config mode
                        if let ConfigurationMode::OllamaServer { testing, .. } =
                            &mut self.configuration_mode
                        {
                            *testing = false;
                        }
                    }
                }
            }
            ConfigurationMode::OpenAIKey { .. } => {
                self.add_debug_log(
                    crate::ui::debug::LogLevel::Warning,
                    "[Config] OpenAI provider not yet implemented".to_string(),
                );
                self.configuration_mode = ConfigurationMode::None;
            }
            ConfigurationMode::AnthropicKey { .. } => {
                self.add_debug_log(
                    crate::ui::debug::LogLevel::Warning,
                    "[Config] Anthropic provider not yet implemented".to_string(),
                );
                self.configuration_mode = ConfigurationMode::None;
            }
            ConfigurationMode::None => {}
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
