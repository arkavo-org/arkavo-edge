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
use crate::ui::{TaskManager, chat::ChatView, code::CodeView, dataflow::DataflowView, debug::DebugView, diff::DiffView};
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
            available_models: vec![
                "llava:7b".to_string(),
                "devstral:latest".to_string(),
                "deepseek-r1:14b".to_string(),
                "qwen3:0.6b".to_string(),
                "dolphin3:latest".to_string(),
            ],
            selected_model: 0,
            input_focused: true,
            last_quit_attempt: None,
            active_model: Some("devstral:latest".to_string()),
            main_task_id: None,
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
            available_models: vec![
                "llava:7b".to_string(),
                "devstral:latest".to_string(),
                "deepseek-r1:14b".to_string(),
                "qwen3:0.6b".to_string(),
                "dolphin3:latest".to_string(),
            ],
            selected_model: 0,
            input_focused: true,
            last_quit_attempt: None,
            active_model: Some("devstral:latest".to_string()),
            main_task_id: None,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;

        // Setup terminal
        self.setup_terminal()?;

        // Ollama configuration is handled by arkavo chat command

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

        // Don't auto-launch Helix to prevent blank screen on startup
        // User can press 'e' when ready

        loop {
            // Check for LLM responses - drain all available messages
            let mut responses_to_process = Vec::new();
            if let Some(ref mut llm_rx) = self.llm_rx {
                // Collect all available responses first
                while let Ok(response) = llm_rx.try_recv() {
                    responses_to_process.push(response);
                }
            }

            // Process collected responses
            for response in responses_to_process {
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
                        self.add_debug_log(
                            crate::ui::debug::LogLevel::Error,
                            format!("[UI] LLM error for task {}: {}", response.task_id, error),
                        );
                    } else if response.is_streaming && !response.is_complete {
                        // Handle streaming chunk
                        task.set_status(crate::ui::task_window::TaskStatus::Streaming);

                        // Check if we need to start a new assistant message
                        if task
                            .messages
                            .back()
                            .map(|m| m.role != crate::ui::task_window::MessageRole::Assistant)
                            .unwrap_or(true)
                        {
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

            // Always check for events but don't block
            if event::poll(Duration::from_millis(16))? {
                // ~60fps for smooth updates
                match event::read()? {
                    Event::Key(key) => {
                        self.telemetry.track_key_event();
                        match key.code {
                            KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                self.should_quit = true;
                            }
                            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::ALT) => {
                                self.vim_enabled = !self.vim_enabled;
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
                                // Tab key disabled - model switching not yet implemented
                                // See issue #86 for multi-model support
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
                                            if let Some(ref active) = self.active_model {
                                                active.clone()
                                            } else {
                                                "devstral:latest".to_string()
                                            };

                                        // Check if we have a main task, if not create one
                                        let task_id = if let Some(id) = self.main_task_id {
                                            id
                                        } else {
                                            let id = uuid::Uuid::new_v4();
                                            let task = self.task_manager.create_task(
                                                "LLM Conversation".to_string(),
                                                displayed_model.clone(),
                                            );
                                            task.id = id;
                                            self.main_task_id = Some(id);
                                            self.task_manager.active_task = Some(0); // Make it the active task
                                            id
                                        };

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
                            // Press Ctrl+I to toggle input/scroll mode
                            KeyCode::Char('i') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                self.input_focused = !self.input_focused;
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
                                }
                            }
                            KeyCode::Backspace if self.input_focused => {
                                // Remove last character from input buffer
                                self.input_buffer.pop();
                            }
                            _ => {
                                let event = AppEvent::from_crossterm_event(Event::Key(key));
                                self.handle_event(event)?;
                            }
                        }
                    }
                    Event::Resize(_, _) => {
                        // Terminal will be redrawn in the main loop
                    }
                    _ => {}
                }
            }

            if self.should_quit {
                break;
            }

            // Always render to ensure UI stays consistent
            // Force full redraw when input is focused to prevent artifacts
            if self.input_focused {
                terminal.draw(|f| {
                    // Clear the entire frame first when input is active
                    f.render_widget(ratatui::widgets::Clear, f.area());
                    self.render(f);
                })?;
            } else {
                terminal.draw(|f| self.render(f))?;
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
                Constraint::Length(3), // Input area at top
                Constraint::Length(3), // Model selector
                Constraint::Min(10),   // Task windows
                Constraint::Length(1), // Status bar
            ])
            .split(frame.area());

        // Render input area at the top
        let input_title = if self.input_focused {
            " Input (Press Ctrl+E for Helix) "
        } else {
            " Input (Press 'i' to focus) "
        };

        let input_block = Block::default()
            .title(input_title)
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
        let buffer_len = self.input_buffer.len();

        let (input_text, is_placeholder) = if self.input_buffer.is_empty() {
            (
                "Type your prompt here or press Ctrl+E to open Helix editor...".to_string(),
                true,
            )
        } else {
            // Show only the visible portion of the input buffer with horizontal scrolling
            let scroll_offset = buffer_len.saturating_sub(visible_width);
            (
                self.input_buffer
                    .chars()
                    .skip(scroll_offset)
                    .collect::<String>(),
                false,
            )
        };

        let input_paragraph = Paragraph::new(input_text).style(if is_placeholder {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default()
        });
        frame.render_widget(input_paragraph, input_inner);

        // Handle cursor visibility and position
        if self.input_focused {
            // Calculate visible portion of input buffer
            let visible_width = input_inner.width.saturating_sub(1) as usize;
            let buffer_len = self.input_buffer.len();

            // Calculate scroll offset to keep cursor visible
            let scroll_offset = buffer_len.saturating_sub(visible_width);

            // Calculate cursor position within visible area
            let cursor_offset = buffer_len.saturating_sub(scroll_offset);
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
            "Active Model: devstral:latest".to_string()
        };

        // Create a block for the model selector area
        let model_selector_block = Block::default()
            .borders(Borders::ALL)
            .title(active_model_display)
            .border_style(Style::default().fg(Color::Yellow));

        let model_inner = model_selector_block.inner(chunks[1]);
        frame.render_widget(model_selector_block, chunks[1]);

        // Add info text about model selection
        let info_text = "Model selection coming soon (see issue #86)";
        let info_paragraph = Paragraph::new(info_text)
            .style(Style::default().fg(Color::DarkGray))
            .alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(info_paragraph, model_inner);

        // Render task windows
        if self.task_manager.tasks.is_empty() {
            let help_text = r#"
Welcome to Arkavo Terminal UI

• Type your prompt and press Enter to start a conversation
• Press Ctrl+E to open Helix editor
• Press Ctrl+I to toggle input/scroll mode
• Press Ctrl+Q to quit

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

        let active_model = if let Some(ref model) = self.active_model {
            model.clone()
        } else {
            "devstral:latest".to_string()
        };

        let mode_indicator = if self.input_focused {
            "INPUT MODE"
        } else {
            "SCROLL MODE (↑↓/jk/PgUp/PgDn/Home/End)"
        };

        let status = format!(
            " {mode_indicator} | Model: {active_model} | Ctrl+E: Helix | Enter: Send | Ctrl+I: Toggle Mode | Ctrl+Q: Quit "
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
            .map(|m| m.is_streaming)
            .unwrap_or(false);

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
                std::io::stdout(),
                terminal::EnterAlternateScreen,
                terminal::Clear(terminal::ClearType::All),
                cursor::Hide,
                cursor::RestorePosition
            );

            // Add a small delay for terminal to stabilize
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

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
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
