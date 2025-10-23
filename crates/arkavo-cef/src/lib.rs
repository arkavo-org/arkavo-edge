pub mod dom_commands;
pub mod error;
pub mod process;
pub mod protocol;
pub mod uds;

pub use dom_commands::{DOMCommandBuilder, DOMOp};
pub use error::{CefError, Result};
pub use process::CefProcess;
pub use protocol::DOMEvent;
pub use uds::{ReceivedMessage, UdsTransport};

use std::path::Path;
use tokio::time::Duration;
use tracing::info;

pub struct CefRenderer {
    process: CefProcess,
    commands: Option<DOMCommandBuilder>,
}

impl CefRenderer {
    pub async fn new(renderer_path: impl AsRef<Path>) -> Result<Self> {
        let temp_dir = std::env::temp_dir();
        let socket_path = temp_dir.join(format!("arkavo_dom_{}.sock", std::process::id()));

        info!("Starting CEF renderer at {:?}", renderer_path.as_ref());

        let mut process = CefProcess::spawn(&socket_path, renderer_path)?;

        process.wait_for_socket(Duration::from_secs(10)).await?;

        let transport = UdsTransport::connect(&socket_path).await?;
        let commands = DOMCommandBuilder::new(transport);

        info!("CEF renderer initialized successfully");

        Ok(Self {
            process,
            commands: Some(commands),
        })
    }

    /// Returns a mutable reference to the DOM command builder.
    ///
    /// # Panics
    ///
    /// Panics if commands have not been initialized.
    pub fn commands(&mut self) -> &mut DOMCommandBuilder {
        self.commands.as_mut().expect("Commands not initialized")
    }

    /// Attempts to receive an event from the CEF renderer (non-blocking).
    ///
    /// Returns `Ok(Some(event))` if an event was received, `Ok(None)` if no event is available.
    pub async fn try_recv_event(&mut self) -> Result<Option<DOMEvent>> {
        if let Some(commands) = &mut self.commands {
            commands.try_recv_event().await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cef_renderer_creation() {
        let result = CefRenderer::new("/nonexistent/path").await;
        assert!(result.is_err());
    }
}
