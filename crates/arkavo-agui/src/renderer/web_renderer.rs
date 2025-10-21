use super::UiRenderer;
use anyhow::Result;

pub struct WebRenderer {
    html_content: String,
    css_content: String,
    js_content: String,
}

impl WebRenderer {
    pub async fn new() -> Result<Self> {
        Ok(Self {
            html_content: String::new(),
            css_content: String::new(),
            js_content: String::new(),
        })
    }
}

#[async_trait::async_trait]
impl UiRenderer for WebRenderer {
    async fn render(&mut self, html: &str, css: &str, js: &str) -> Result<()> {
        self.html_content = html.to_string();
        self.css_content = css.to_string();
        self.js_content = js.to_string();
        tracing::debug!("Web renderer: Updated content");
        Ok(())
    }

    async fn update_element(&mut self, selector: &str, html: &str) -> Result<()> {
        tracing::debug!("Web renderer: Update element {} with {}", selector, html);
        Ok(())
    }

    async fn set_style(&mut self, selector: &str, property: &str, value: &str) -> Result<()> {
        tracing::debug!(
            "Web renderer: Set style {} on {} to {}",
            property,
            selector,
            value
        );
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
        true
    }

    async fn shutdown(self: Box<Self>) -> Result<()> {
        tracing::info!("Web renderer shutdown");
        Ok(())
    }
}
