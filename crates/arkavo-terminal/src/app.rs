use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
};
use std::io;
use std::time::Duration;

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
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;

        // Setup terminal
        self.setup_terminal()?;

        // Run app with panic recovery
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // We need to block on the async operation since catch_unwind doesn't work with async
            let runtime = tokio::runtime::Handle::current();
            runtime.block_on(self.run_app(&mut terminal))
        }));

        // Always restore terminal state, even on panic
        self.restore_terminal()?;

        match res {
            Ok(result) => result,
            Err(panic) => {
                eprintln!("Terminal UI panicked! Terminal state has been restored.");
                std::panic::resume_unwind(panic);
            }
        }
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

            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    match key.code {
                        KeyCode::Char('q') => self.should_quit = true,
                        KeyCode::Tab => self.switch_view(),
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
