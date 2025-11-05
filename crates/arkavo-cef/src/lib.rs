pub mod async_dom_commands;
pub mod async_transport;
pub mod command_tracker;
pub mod dom_commands;
pub mod error;
pub mod health_reporter;
pub mod process;
pub mod protocol;
pub mod uds;

pub use async_dom_commands::{AsyncDOMCommandBuilder, DOMOp as AsyncDOMOp};
pub use async_transport::AsyncTransport;
pub use command_tracker::{CommandHealthReport, CommandHealthSnapshot, CommandTracker};
pub use dom_commands::{DOMCommandBuilder, DOMOp};
pub use error::{CefError, Result};
pub use health_reporter::CefHealthReporter;
pub use process::CefProcess;
pub use protocol::{DOMError, DOMEvent};

// Export both sync and async renderer types
pub use CefRenderer as SyncCefRenderer;

// Re-export uds module for external access
pub use uds::{ReceivedMessage, UdsTransport};

use std::path::Path;
use std::sync::Arc;
use tokio::time::Duration;
use tracing::info;

pub struct CefRenderer {
    process: CefProcess,
    commands: Option<DOMCommandBuilder>,
    tracker: Arc<CommandTracker>,
}

impl CefRenderer {
    pub async fn new(renderer_path: impl AsRef<Path>) -> Result<Self> {
        let temp_dir = std::env::temp_dir();
        let socket_path = temp_dir.join(format!("arkavo_dom_{}.sock", std::process::id()));

        info!("Starting CEF renderer at {:?}", renderer_path.as_ref());

        let mut process = CefProcess::spawn(&socket_path, renderer_path)?;

        process.wait_for_socket(Duration::from_secs(10)).await?;

        let transport = UdsTransport::connect(&socket_path).await?;
        let tracker = Arc::new(CommandTracker::new());
        let commands = DOMCommandBuilder::with_tracker(transport, tracker.clone());

        info!("CEF renderer initialized successfully");

        Ok(Self {
            process,
            commands: Some(commands),
            tracker,
        })
    }

    pub fn tracker(&self) -> Arc<CommandTracker> {
        self.tracker.clone()
    }

    pub fn commands(&mut self) -> &mut DOMCommandBuilder {
        self.commands.as_mut().expect("Commands not initialized")
    }

    pub async fn try_recv_event(&mut self) -> Result<Option<DOMEvent>> {
        if let Some(commands) = &mut self.commands {
            commands.try_recv_event().await
        } else {
            Ok(None)
        }
    }

    pub async fn try_recv_error(&mut self) -> Result<Option<DOMError>> {
        if let Some(commands) = &mut self.commands {
            commands.try_recv_error().await
        } else {
            Ok(None)
        }
    }

    pub fn is_running(&mut self) -> bool {
        self.process.is_running()
    }

    pub fn shutdown(mut self) -> Result<()> {
        self.process.kill()
    }
}

pub struct AsyncCefRenderer {
    process: CefProcess,
    commands: Option<AsyncDOMCommandBuilder>,
    tracker: Arc<CommandTracker>,
}

impl AsyncCefRenderer {
    pub async fn new(renderer_path: impl AsRef<Path>) -> Result<Self> {
        let temp_dir = std::env::temp_dir();
        let socket_path = temp_dir.join(format!("arkavo_dom_{}.sock", std::process::id()));

        info!(
            "Starting CEF renderer (async) at {:?}",
            renderer_path.as_ref()
        );

        let mut process = CefProcess::spawn(&socket_path, renderer_path)?;

        process.wait_for_socket(Duration::from_secs(10)).await?;

        let transport = AsyncTransport::connect(&socket_path).await?;
        let tracker = Arc::new(CommandTracker::new());
        let commands = AsyncDOMCommandBuilder::with_tracker(transport, tracker.clone());

        info!("Async CEF renderer initialized successfully");

        Ok(Self {
            process,
            commands: Some(commands),
            tracker,
        })
    }

    pub fn tracker(&self) -> Arc<CommandTracker> {
        self.tracker.clone()
    }

    pub fn commands(&mut self) -> &mut AsyncDOMCommandBuilder {
        self.commands.as_mut().expect("Commands not initialized")
    }

    pub fn try_recv_event(&mut self) -> Option<DOMEvent> {
        if let Some(commands) = &mut self.commands {
            commands.try_recv_event()
        } else {
            None
        }
    }

    pub fn is_running(&mut self) -> bool {
        self.process.is_running()
    }

    pub fn shutdown(mut self) -> Result<()> {
        self.process.kill()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cef_renderer_creation() {
        let result = CefRenderer::new("/nonexistent/path").await;
        assert!(result.is_err());
    }
}
