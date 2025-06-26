pub mod app;
pub mod benchmark;
pub mod event;
pub mod multi_terminal;
pub mod renderer;
pub mod ui;

#[cfg(test)]
mod tests;

use anyhow::Result;

pub use app::App;
pub use event::{AppEvent, EventHandler};
pub use multi_terminal::{MultiTerminalManager, TaskType, TerminalSpawnConfig};
pub use renderer::{DiffRenderer, RenderMetrics, Renderable};

pub async fn run() -> Result<()> {
    let mut app = App::new();
    app.run().await
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
            frame_budget_ms: 50, // Target <50ms render time
            enable_mouse: true,
            enable_alternate_screen: true,
            max_fps: 60,
        }
    }
}
