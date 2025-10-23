use crate::types::{UIContent, UIEvent, UserInterface};
use anyhow::Result;
use tokio::sync::mpsc;

pub struct WebAdapter {
    port: u16,
    event_rx: Option<mpsc::Receiver<UIEvent>>,
    running: bool,
}

impl WebAdapter {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            event_rx: None,
            running: false,
        }
    }
}

#[async_trait::async_trait]
impl UserInterface for WebAdapter {
    async fn initialize(&mut self) -> Result<()> {
        let (_event_tx, event_rx) = mpsc::channel(100);
        self.event_rx = Some(event_rx);
        self.running = true;

        tracing::info!("Web UI adapter initialized on port {}", self.port);
        tracing::info!(
            "Note: Web UI is launched directly via use_web_gateway(), not through adapter"
        );

        Ok(())
    }

    async fn display(&mut self, content: UIContent) -> Result<()> {
        match content {
            UIContent::Html {
                html,
                css: _,
                js: _,
            } => {
                tracing::debug!("Web UI: Display HTML ({} bytes)", html.len());
            }
            UIContent::Text(text) | UIContent::Markdown(text) => {
                tracing::debug!("Web UI: Display text ({} chars)", text.len());
            }
            UIContent::StreamChunk {
                content,
                is_complete,
            } => {
                tracing::debug!(
                    "Web UI: Stream chunk ({} chars, complete: {})",
                    content.len(),
                    is_complete
                );
            }
            UIContent::Error(error) => {
                tracing::error!("Web UI: Display error: {}", error);
            }
            UIContent::StatusUpdate(status) => {
                tracing::info!("Web UI: Status update: {}", status);
            }
        }

        Ok(())
    }

    async fn next_event(&mut self) -> Result<UIEvent> {
        if let Some(ref mut rx) = self.event_rx
            && let Some(event) = rx.recv().await
        {
            return Ok(event);
        }

        Ok(UIEvent::Shutdown)
    }

    fn is_running(&self) -> bool {
        self.running
    }

    async fn shutdown(mut self: Box<Self>) -> Result<()> {
        self.running = false;
        tracing::info!("Web UI shutting down");
        Ok(())
    }
}
