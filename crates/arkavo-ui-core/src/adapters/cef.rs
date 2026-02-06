use crate::types::{UIContent, UIEvent, UserInterface};
use anyhow::Result;
use arkavo_agui::UiRenderer;
use arkavo_agui::renderer::cef_renderer::CefRendererImpl;
use std::collections::HashMap;

fn encode_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

pub struct CEFAdapter {
    renderer: CefRendererImpl,
}

impl CEFAdapter {
    pub async fn new() -> Result<Self> {
        let renderer = CefRendererImpl::new().await?;
        Ok(Self { renderer })
    }
}

#[async_trait::async_trait]
impl UserInterface for CEFAdapter {
    async fn initialize(&mut self) -> Result<()> {
        tracing::info!("CEF renderer initialized, waiting for page load...");
        tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;
        Ok(())
    }

    async fn display(&mut self, content: UIContent) -> Result<()> {
        match content {
            UIContent::Html { html, css, js } => {
                self.renderer.render(&html, &css, &js).await?;
            }
            UIContent::Text(text) | UIContent::Markdown(text) => {
                let html = format!(
                    r#"<div style="padding: 40px; font-family: system-ui, -apple-system, sans-serif; white-space: pre-wrap;">{}</div>"#,
                    encode_html(&text)
                );
                self.renderer.render(&html, "", "").await?;
            }
            UIContent::StreamChunk {
                content,
                is_complete: _,
            } => {
                let html = format!(
                    r#"<div style="padding: 40px; font-family: system-ui, -apple-system, sans-serif; white-space: pre-wrap;">{}</div>"#,
                    encode_html(&content)
                );
                self.renderer.render(&html, "", "").await?;
            }
            UIContent::Error(error) => {
                let html = format!(
                    r#"<div style="padding: 40px; font-family: system-ui; color: #d32f2f; background: #ffebee; border-radius: 8px;">{}</div>"#,
                    encode_html(&error)
                );
                self.renderer.render(&html, "", "").await?;
            }
            UIContent::StatusUpdate(status) => {
                let html = format!(
                    r#"<div style="padding: 12px 16px; font-family: system-ui; color: #666; background: #f5f5f5; border-radius: 4px; font-size: 12px;">{}</div>"#,
                    encode_html(&status)
                );
                self.renderer
                    .update_element("#status", &html)
                    .await
                    .unwrap_or(());
            }
        }
        Ok(())
    }

    async fn next_event(&mut self) -> Result<UIEvent> {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            if !self.renderer.is_running() {
                return Ok(UIEvent::Shutdown);
            }

            if let Some(event) = self.renderer.try_recv_event().await? {
                if event.event_type == "submit" && !event.value.is_empty() {
                    return Ok(UIEvent::TextInput(event.value));
                } else if event.event_type == "click" {
                    return Ok(UIEvent::ButtonClick {
                        id: event.target_id,
                        label: event.value,
                    });
                } else if event.event_type == "form_submit" {
                    let data: HashMap<String, String> =
                        serde_json::from_str(&event.data).unwrap_or_default();
                    return Ok(UIEvent::FormSubmit { data });
                }
            }
        }
    }

    fn is_running(&self) -> bool {
        self.renderer.is_running()
    }

    async fn shutdown(self: Box<Self>) -> Result<()> {
        Box::new(self.renderer).shutdown().await
    }
}
