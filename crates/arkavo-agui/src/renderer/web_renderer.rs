use super::UiRenderer;
use anyhow::Result;
use tokio::sync::broadcast;

pub struct WebRenderer {
    html_content: String,
    css_content: String,
    js_content: String,
    broadcast_tx: Option<broadcast::Sender<String>>,
}

impl WebRenderer {
    pub async fn new() -> Result<Self> {
        Ok(Self {
            html_content: String::new(),
            css_content: String::new(),
            js_content: String::new(),
            broadcast_tx: None,
        })
    }

    pub fn with_broadcast(mut self, tx: broadcast::Sender<String>) -> Self {
        self.broadcast_tx = Some(tx);
        self
    }
}

#[async_trait::async_trait]
impl UiRenderer for WebRenderer {
    async fn render(&mut self, html: &str, css: &str, js: &str) -> Result<()> {
        self.html_content = html.to_string();
        self.css_content = css.to_string();
        self.js_content = js.to_string();

        if let Some(ref tx) = self.broadcast_tx {
            let message = serde_json::json!({
                "type": "render",
                "html": html,
                "css": css,
                "js": js
            })
            .to_string();

            let _ = tx.send(message);
            tracing::debug!(
                "Web renderer: Broadcasted render update ({} bytes)",
                html.len()
            );
        } else {
            tracing::debug!("Web renderer: Updated content (no broadcast channel)");
        }

        Ok(())
    }

    async fn update_element(&mut self, selector: &str, html: &str) -> Result<()> {
        if let Some(ref tx) = self.broadcast_tx {
            let message = serde_json::json!({
                "type": "update_element",
                "selector": selector,
                "html": html
            })
            .to_string();

            let _ = tx.send(message);
            tracing::debug!("Web renderer: Broadcasted element update for {}", selector);
        }

        Ok(())
    }

    async fn set_style(&mut self, selector: &str, property: &str, value: &str) -> Result<()> {
        if let Some(ref tx) = self.broadcast_tx {
            let message = serde_json::json!({
                "type": "set_style",
                "selector": selector,
                "property": property,
                "value": value
            })
            .to_string();

            let _ = tx.send(message);
            tracing::debug!("Web renderer: Broadcasted style update for {}", selector);
        }

        Ok(())
    }

    async fn add_event_listener(&mut self, selector: &str, event_type: &str) -> Result<()> {
        tracing::debug!(
            "Web renderer: Add event listener {} on {}",
            event_type,
            selector
        );
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.broadcast_tx.is_some()
    }

    async fn shutdown(self: Box<Self>) -> Result<()> {
        tracing::info!("Web renderer shutdown");
        Ok(())
    }
}
