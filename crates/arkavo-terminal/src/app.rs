use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
};
use std::io;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::event::{AppEvent, EventHandler};
use crate::renderer::Renderable;
use crate::ui::{chat::ChatView, code::CodeView, diff::DiffView};

pub enum ViewMode {
    Chat,
    Code,
    Diff,
}

pub struct App {
    pub should_quit: bool,
    pub view_mode: ViewMode,
    pub chat_view: ChatView,
    pub code_view: CodeView,
    pub diff_view: DiffView,
    pub event_handler: EventHandler,
    pub ui_tx: Option<mpsc::Sender<String>>,
    pub llm_rx: Option<mpsc::Receiver<String>>,
}

impl App {
    pub fn new() -> Self {
        Self {
            should_quit: false,
            view_mode: ViewMode::Chat,
            chat_view: ChatView::new(),
            code_view: CodeView::new(),
            diff_view: DiffView::new(),
            event_handler: EventHandler::new(),
            ui_tx: None,
            llm_rx: None,
        }
    }

    pub fn new_with_channels(ui_tx: mpsc::Sender<String>, llm_rx: mpsc::Receiver<String>) -> Self {
        Self {
            should_quit: false,
            view_mode: ViewMode::Chat,
            chat_view: ChatView::new(),
            code_view: CodeView::new(),
            diff_view: DiffView::new(),
            event_handler: EventHandler::new(),
            ui_tx: Some(ui_tx),
            llm_rx: Some(llm_rx),
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;

        // Setup terminal
        self.setup_terminal()?;

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
        loop {
            terminal.draw(|f| self.render(f))?;

            // Check for LLM responses
            if let Some(ref mut llm_rx) = self.llm_rx {
                match llm_rx.try_recv() {
                    Ok(response) => {
                        // Finish any streaming message and add the complete response
                        self.chat_view.finish_streaming();
                        self.chat_view
                            .add_message(crate::ui::chat::MessageRole::Assistant, response);
                    }
                    Err(_) => {}
                }
            }

            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    match key.code {
                        KeyCode::Char('q') => self.should_quit = true,
                        KeyCode::Tab => self.switch_view(),
                        KeyCode::Enter => {
                            // Handle Enter key specially for chat view
                            if matches!(self.view_mode, ViewMode::Chat)
                                && !self.chat_view.input_buffer.is_empty()
                            {
                                let input = self.chat_view.input_buffer.clone();

                                // Send to LLM if channel is available
                                if let Some(ref ui_tx) = self.ui_tx {
                                    let _ = ui_tx.try_send(input.clone());

                                    // Add user message to chat
                                    self.chat_view
                                        .add_message(crate::ui::chat::MessageRole::User, input);
                                    self.chat_view.input_buffer.clear();

                                    // Add "thinking" message
                                    self.chat_view.start_streaming_message(
                                        crate::ui::chat::MessageRole::Assistant,
                                    );
                                } else {
                                    // Fallback to echo mode if no channel
                                    let event = AppEvent::from_crossterm_event(Event::Key(key));
                                    self.handle_event(event)?;
                                }
                            }
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
        }

        Ok(())
    }

    fn render(&mut self, frame: &mut ratatui::Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Status bar
                Constraint::Min(0),    // Main content
                Constraint::Length(1), // Input line
            ])
            .split(frame.area());

        match self.view_mode {
            ViewMode::Chat => self.chat_view.render(frame, chunks[1]),
            ViewMode::Code => self.code_view.render(frame, chunks[1]),
            ViewMode::Diff => self.diff_view.render(frame, chunks[1]),
        }
    }

    fn switch_view(&mut self) {
        self.view_mode = match self.view_mode {
            ViewMode::Chat => ViewMode::Code,
            ViewMode::Code => ViewMode::Diff,
            ViewMode::Diff => ViewMode::Chat,
        };
    }

    fn handle_event(&mut self, event: AppEvent) -> Result<()> {
        match self.view_mode {
            ViewMode::Chat => self.chat_view.handle_event(event),
            ViewMode::Code => self.code_view.handle_event(event),
            ViewMode::Diff => self.diff_view.handle_event(event),
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
