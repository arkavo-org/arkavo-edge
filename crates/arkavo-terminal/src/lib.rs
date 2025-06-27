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

pub struct TerminalContext {
    pub message_tx: mpsc::Sender<ChatMessage>,
    pub message_rx: mpsc::Receiver<ChatMessage>,
}

pub async fn run() -> Result<()> {
    // Create dummy channels for standalone mode
    let (_tx, rx) = mpsc::channel::<String>(1);
    let (tx, _rx) = mpsc::channel::<String>(1);
    run_with_channels(tx, rx).await
}

pub async fn run_with_channels(
    ui_tx: mpsc::Sender<String>,
    llm_rx: mpsc::Receiver<String>,
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

pub async fn run_task_view(task_id: &str, session_id: &str) -> Result<()> {
    // TODO: Implement task-specific view that connects to main process
    let mut app = App::new();
    println!(
        "Running task view for task: {} in session: {}",
        task_id, session_id
    );
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
