use anyhow::Result;
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
use std::io;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::event::{AppEvent, EventHandler};
use crate::helix::HelixEditor;
use crate::renderer::Renderable;
use crate::telemetry::UITelemetry;
use crate::ui::{chat::ChatView, code::CodeView, debug::DebugView, diff::DiffView, TaskManager};
use crate::vim::VimState;

pub enum ViewMode {
    Chat,
    Code,
    Diff,
    Debug,
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

pub struct App {
    pub should_quit: bool,
    pub view_mode: ViewMode,
    pub layout_mode: LayoutMode,
    pub focused_pane: FocusedPane,
    pub chat_view: ChatView,
    pub code_view: CodeView,
    pub diff_view: DiffView,
    pub debug_view: DebugView,
    pub thinking_view: ChatView, // Reuse ChatView for chain-of-thought display
    pub event_handler: EventHandler,
    pub ui_tx: Option<mpsc::Sender<String>>,
    pub llm_rx: Option<mpsc::Receiver<String>>,
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
            available_models: vec![
                "llava:7b".to_string(),
                "devstral:latest".to_string(),
                "deepseek-r1:14b".to_string(),
                "qwen3:0.6b".to_string(),
                "dolphin3:latest".to_string(),
            ],
            selected_model: 0,
        }
    }

    pub fn new_with_channels(ui_tx: mpsc::Sender<String>, llm_rx: mpsc::Receiver<String>) -> Self {
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
            available_models: vec![
                "llava:7b".to_string(),
                "devstral:latest".to_string(),
                "deepseek-r1:14b".to_string(),
                "qwen3:0.6b".to_string(),
                "dolphin3:latest".to_string(),
            ],
            selected_model: 0,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;

        // Setup terminal
        self.setup_terminal()?;

        // Configure Ollama from saved settings and fetch models on startup
        self.configure_ollama_from_memory().await;
        self.fetch_ollama_models().await;

        // Run the app
        let result = self.run_app(&mut terminal).await;

        // Always restore terminal state
        self.restore_terminal()?;

        result
    }

    fn setup_terminal(&self) -> Result<()> {
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(
            io::stdout(),
            crossterm::terminal::EnterAlternateScreen,
            crossterm::event::EnableMouseCapture
        )?;
        Ok(())
    }

    fn restore_terminal(&self) -> Result<()> {
        // Ignore errors during cleanup to ensure we try all steps
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture
        );
        Ok(())
    }

    async fn run_app<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<()> {
        // Immediate first draw for fast startup
        terminal.draw(|f| self.render(f))?;
        
        // Auto-launch Helix editor on startup
        if self.helix_editor.is_some() {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            self.launch_helix_editor().await;
        }

        loop {
            // Check for LLM responses - drain all available messages
            let mut debug_messages = Vec::new();
            if let Some(ref mut llm_rx) = self.llm_rx {
                // Process all available messages to prevent message loss
                while let Ok(response) = llm_rx.try_recv() {
                    if response == "<<STREAM_START>>" {
                        // Start streaming a new assistant message
                        debug_messages.push((
                            crate::ui::debug::LogLevel::Info,
                            "[UI] Started streaming response".to_string(),
                        ));
                        self.chat_view
                            .start_streaming_message(crate::ui::chat::MessageRole::Assistant);
                    } else if let Some(chunk) = response.strip_prefix("<<STREAM_CHUNK>>") {
                        // Append chunk to streaming message
                        debug_messages.push((
                            crate::ui::debug::LogLevel::Debug,
                            format!("[UI] Received chunk: {} chars", chunk.len()),
                        ));
                        self.chat_view.append_to_streaming(chunk);
                    } else if response == "<<STREAM_END>>" {
                        // Finish streaming
                        debug_messages.push((
                            crate::ui::debug::LogLevel::Info,
                            "[UI] Finished streaming response".to_string(),
                        ));
                        self.chat_view.finish_streaming();
                        self.telemetry.track_message_received();
                    } else if let Some(error_msg) = response.strip_prefix("<<STREAM_ERROR>>") {
                        // Handle streaming error
                        debug_messages.push((
                            crate::ui::debug::LogLevel::Error,
                            format!("[UI] Streaming error: {}", error_msg),
                        ));
                        self.chat_view.finish_streaming();
                        self.chat_view.add_message(
                            crate::ui::chat::MessageRole::System,
                            error_msg.to_string(),
                        );
                    } else {
                        // Fallback for non-streaming messages
                        debug_messages.push((
                            crate::ui::debug::LogLevel::Info,
                            format!(
                                "[UI] Received non-streaming response: {} chars",
                                response.len()
                            ),
                        ));
                        self.chat_view.finish_streaming();
                        self.chat_view
                            .add_message(crate::ui::chat::MessageRole::Assistant, response);
                        self.telemetry.track_message_received();
                    }
                }
            }

            // Add collected debug messages
            for (level, message) in debug_messages {
                self.add_debug_log(level, message);
            }

            // Check for thinking/chain-of-thought updates
            if let Some(ref mut thinking_rx) = self.thinking_rx {
                while let Ok(thought) = thinking_rx.try_recv() {
                    self.thinking_view
                        .add_message(crate::ui::chat::MessageRole::System, thought);
                }
            }

            // Reduce polling interval for better responsiveness
            if event::poll(Duration::from_millis(8))? {
                // ~120fps for smooth updates
                if let Event::Key(key) = event::read()? {
                    self.telemetry.track_key_event();
                    match key.code {
                        KeyCode::Char('q') => self.should_quit = true,
                        KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::ALT) => {
                            self.vim_enabled = !self.vim_enabled;
                            self.add_debug_log(
                                crate::ui::debug::LogLevel::Info,
                                format!("[UI] Vim mode {}", if self.vim_enabled { "enabled" } else { "disabled" }),
                            );
                        }
                        KeyCode::Char('e') => {
                            // Simple 'e' key launches Helix on macOS
                            self.launch_helix_editor().await;
                        }
                        KeyCode::Tab => {
                            // Tab switches to next model/agent
                            self.selected_model = (self.selected_model + 1) % self.available_models.len();
                            self.add_debug_log(
                                crate::ui::debug::LogLevel::Info,
                                format!("[UI] Selected model: {}", self.available_models[self.selected_model]),
                            );
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
                            // Create a new task with the input buffer content
                            if !self.input_buffer.is_empty() {
                                let model_name = self.available_models[self.selected_model].clone();
                                let task = self.task_manager.create_task(
                                    "LLM Agent".to_string(),
                                    model_name.clone()
                                );
                                
                                task.add_message(
                                    crate::ui::task_window::MessageRole::User,
                                    self.input_buffer.clone()
                                );
                                task.set_status(crate::ui::task_window::TaskStatus::Processing);
                                
                                // Send to LLM if channel is available
                                if let Some(ref ui_tx) = self.ui_tx {
                                    let _ = ui_tx.try_send(self.input_buffer.clone());
                                }
                                
                                self.input_buffer.clear();
                                self.telemetry.track_message_sent();
                            }
                        }
                        KeyCode::Char(c) => {
                            // Add character to input buffer
                            self.input_buffer.push(c);
                        }
                        KeyCode::Backspace => {
                            // Remove last character from input buffer
                            self.input_buffer.pop();
                        }
                        _ => {
                            let event = AppEvent::from_crossterm_event(Event::Key(key));
                            self.handle_event(event)?;
                        }
                    }
                }
            }

            if self.should_quit {
                break;
            }

            // Render after processing events/messages
            terminal.draw(|f| self.render(f))?;

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

    fn render(&mut self, frame: &mut ratatui::Frame) {
        // Always use the new task-based layout
        self.render_task_layout(frame);
    }

    fn render_task_layout(&mut self, frame: &mut ratatui::Frame) {
        use ratatui::style::{Color, Style};
        use ratatui::widgets::{Block, Borders, Paragraph};
        
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // Input area at top
                Constraint::Length(3),  // Model selector
                Constraint::Min(10),    // Task windows
                Constraint::Length(1),  // Status bar
            ])
            .split(frame.area());

        // Render input area at the top
        let input_block = Block::default()
            .title(" Input (Press 'e' for Helix) ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));
        let input_inner = input_block.inner(chunks[0]);
        frame.render_widget(input_block, chunks[0]);
        
        let input_text = if self.input_buffer.is_empty() {
            "Type your prompt here or press 'e' to open Helix editor..."
        } else {
            &self.input_buffer
        };
        
        let input_paragraph = Paragraph::new(input_text)
            .style(if self.input_buffer.is_empty() {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            });
        frame.render_widget(input_paragraph, input_inner);

        // Render model selector
        let model_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(
                self.available_models
                    .iter()
                    .map(|_| Constraint::Percentage((100 / self.available_models.len()) as u16))
                    .collect::<Vec<_>>()
            )
            .split(chunks[1]);
            
        for (i, (model, area)) in self.available_models.iter().zip(model_chunks.iter()).enumerate() {
            let is_selected = i == self.selected_model;
            let model_block = Block::default()
                .borders(if is_selected { Borders::ALL } else { Borders::NONE })
                .border_style(Style::default().fg(if is_selected { Color::Cyan } else { Color::DarkGray }));
            
            let model_text = Paragraph::new(model.as_str())
                .style(if is_selected {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::DarkGray)
                })
                .block(model_block);
            
            frame.render_widget(model_text, *area);
        }

        // Render task windows
        if self.task_manager.tasks.is_empty() {
            let help_text = r#"
Welcome to Arkavo Terminal UI

• Press 'e' to open Helix editor
• Type your prompt and press Enter to start a task
• Press Tab to switch between models
• Press 'q' to quit
            "#;
            
            let help_paragraph = Paragraph::new(help_text)
                .style(Style::default().fg(Color::DarkGray))
                .block(Block::default().borders(Borders::ALL).title(" Getting Started "));
            
            frame.render_widget(help_paragraph, chunks[2]);
        } else {
            // Render active tasks in a grid
            let task_count = self.task_manager.tasks.len();
            let cols = ((task_count as f32).sqrt().ceil() as usize).max(1);
            let rows = (task_count + cols - 1) / cols;
            
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
                        self.task_manager.tasks[task_idx].render(frame, *col_chunk);
                        task_idx += 1;
                    }
                }
            }
        }

        // Render status bar
        self.render_status_bar(frame, chunks[3]);
    }

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
        }

        // Render performance metrics
        self.render_performance_bar(frame, chunks[2]);
    }

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

    fn switch_view(&mut self) {
        self.view_mode = match self.view_mode {
            ViewMode::Chat => ViewMode::Code,
            ViewMode::Code => ViewMode::Diff,
            ViewMode::Diff => ViewMode::Debug,
            ViewMode::Debug => ViewMode::Chat,
        };
    }

    fn handle_event(&mut self, event: AppEvent) -> Result<()> {
        match self.layout_mode {
            LayoutMode::Tabbed => match self.view_mode {
                ViewMode::Chat => self.chat_view.handle_event(event),
                ViewMode::Code => self.code_view.handle_event(event),
                ViewMode::Diff => self.diff_view.handle_event(event),
                ViewMode::Debug => self.debug_view.handle_event(event),
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

    async fn fetch_ollama_models(&mut self) {
        // Show loading message
        self.chat_view.add_message(
            crate::ui::chat::MessageRole::System,
            "Fetching available Ollama models...".to_string(),
        );

        // Try to fetch models from Ollama
        match self.list_ollama_models().await {
            Ok(models) if !models.is_empty() => {
                // Get the URL we're connected to
                let base_url = self
                    .get_saved_ollama_url()
                    .await
                    .unwrap_or_else(|_| "http://localhost:11434".to_string());

                let mut message =
                    format!("Connected to Ollama at {}\nAvailable models:\n", base_url);

                // Show models with indication of which will be selected
                let preferred_order = [
                    "qwen3:0.6b", // Specifically prefer the 0.6b model
                    "qwen3",
                    "qwen2.5",
                    "qwen",
                    "llama3.2",
                    "llama3.1",
                    "llama3",
                    "devstral",
                    "mistral",
                    "codestral",
                    "phi3",
                    "gemma",
                    "deepseek",
                ];

                let mut selected = false;
                for (i, model) in models.iter().enumerate() {
                    let model_name = model.split_whitespace().next().unwrap_or(model);

                    // Check if this is a preferred model and the first one found
                    let is_preferred = !selected
                        && preferred_order.iter().any(|&pref| {
                            if pref.contains(':') {
                                // Exact match for specific versions like "qwen3:0.6b"
                                model_name == pref
                            } else {
                                // Contains match for general model names
                                model_name.contains(pref)
                            }
                        });

                    if is_preferred || (!selected && i == 0) {
                        message.push_str(&format!("  • {} ← will be used\n", model));
                        selected = true;
                    } else {
                        message.push_str(&format!("  • {}\n", model));
                    }
                }

                self.chat_view
                    .add_message(crate::ui::chat::MessageRole::System, message);
            }
            Ok(_) => {
                self.chat_view.add_message(
                    crate::ui::chat::MessageRole::System,
                    "No Ollama models found. Make sure Ollama is running.".to_string(),
                );
            }
            Err(e) => {
                // Try to get the actual URL we attempted to use
                let base_url = self
                    .get_saved_ollama_url()
                    .await
                    .unwrap_or_else(|_| "http://localhost:11434".to_string());

                self.chat_view.add_message(
                    crate::ui::chat::MessageRole::System,
                    format!("Failed to list models from Ollama at {}: {}", base_url, e),
                );
            }
        }
    }

    async fn list_ollama_models(&self) -> Result<Vec<String>> {
        use serde::Deserialize;

        #[derive(Debug, Deserialize)]
        struct ModelInfo {
            name: String,
            #[serde(default)]
            size: u64,
        }

        #[derive(Debug, Deserialize)]
        struct ModelsResponse {
            models: Vec<ModelInfo>,
        }

        // Get saved Ollama server configuration from memory
        let base_url = match self.get_saved_ollama_url().await {
            Ok(saved_url) => saved_url,
            Err(_) => "http://localhost:11434".to_string(), // Default to localhost only if no saved config
        };

        let client = reqwest::Client::new();

        let response = client.get(format!("{}/api/tags", base_url)).send().await?;

        if !response.status().is_success() {
            return Ok(vec![]);
        }

        let models_response: ModelsResponse = response.json().await?;
        Ok(models_response
            .models
            .into_iter()
            .map(|m| {
                if m.size > 0 {
                    let size_gb = m.size as f64 / 1_073_741_824.0; // Convert bytes to GB
                    format!("{} ({:.1} GB)", m.name, size_gb)
                } else {
                    m.name
                }
            })
            .collect())
    }

    async fn configure_ollama_from_memory(&self) {
        if let Ok(saved_url) = self.get_saved_ollama_url().await {
            // Set the environment variable for compatibility with LlmClient::from_env()
            unsafe {
                std::env::set_var("OLLAMA_BASE_URL", &saved_url);
            }
        }
    }

    async fn get_saved_ollama_url(&self) -> Result<String> {
        // Check if arkavo-memory with memory feature is available
        #[cfg(feature = "memory")]
        {
            use arkavo_memory::storage::MemoryStorage;
            use std::sync::Arc;

            // Initialize memory storage to check for saved configuration
            let storage = Arc::new(MemoryStorage::new().await?);

            // Try to find saved Ollama server configuration
            let saved_config = storage
                .search("arkavo_ollama_server_config", 10, Some("config"))
                .await?;

            // Find a valid configuration (not cleared)
            if let Some(config) = saved_config
                .into_iter()
                .find(|c| c.memory.content != "CLEARED" && c.memory.content.starts_with("http"))
            {
                return Ok(config.memory.content);
            }
        }

        Err(anyhow::anyhow!("No saved Ollama configuration found"))
    }

    fn render_status_bar(&self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        use ratatui::style::{Color, Style};
        use ratatui::widgets::{Block, Borders, Paragraph};

        let status = format!(
            " Tasks: {} | Model: {} | Tab: Switch Model | e: Helix | Enter: Send | q: Quit ",
            self.task_manager.tasks.len(),
            self.available_models[self.selected_model]
        );

        let paragraph = Paragraph::new(status)
            .style(Style::default().fg(Color::White).bg(Color::DarkGray))
            .block(Block::default().borders(Borders::NONE));

        frame.render_widget(paragraph, area);
    }

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
            .map(|m| m.is_streaming)
            .unwrap_or(false);

        let streaming_indicator = if is_streaming { " ● Streaming" } else { "" };

        let perf_text = format!(
            " {} FPS | Sent: {} | Received: {}{}",
            fps, messages_sent, messages_received, streaming_indicator
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
            // Suspend the terminal temporarily
            let _ = terminal::disable_raw_mode();
            let _ = crossterm::execute!(
                std::io::stdout(),
                terminal::LeaveAlternateScreen,
                cursor::Show
            );
            
            // Get current input buffer content
            let initial_content = self.input_buffer.clone();
            
            // Launch helix and get the edited content
            match helix.launch_with_content(&initial_content) {
                Ok(edited_content) => {
                    // Update the input buffer with edited content
                    self.input_buffer = edited_content.trim_end().to_string();
                    self.add_debug_log(
                        crate::ui::debug::LogLevel::Info,
                        format!("[UI] Helix editor: {} chars pasted", self.input_buffer.len()),
                    );
                }
                Err(e) => {
                    self.add_debug_log(
                        crate::ui::debug::LogLevel::Error,
                        format!("[UI] Helix editor error: {}", e),
                    );
                }
            }
            
            // Restore terminal
            let _ = terminal::enable_raw_mode();
            let _ = crossterm::execute!(
                std::io::stdout(),
                terminal::EnterAlternateScreen,
                cursor::Hide
            );
            
            // Force redraw
            self.chat_view.needs_redraw = true;
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
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
