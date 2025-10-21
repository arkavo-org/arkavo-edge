use super::UiRenderer;
use anyhow::Result;
use arkavo_cef::{CefRenderer, DOMCommandBuilder, DOMEvent};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info};

pub type EventCallback = Arc<dyn Fn(DOMEvent) + Send + Sync>;

pub struct CefRendererImpl {
    renderer: CefRenderer,
    event_callback: Option<EventCallback>,
}

impl CefRendererImpl {
    pub async fn new() -> Result<Self> {
        let renderer_path = Self::find_renderer_binary()?;
        info!("Initializing CEF renderer from {:?}", renderer_path);

        let renderer = CefRenderer::new(renderer_path).await?;

        Ok(Self {
            renderer,
            event_callback: None,
        })
    }

    fn find_renderer_binary() -> Result<PathBuf> {
        if let Ok(path) = std::env::var("ARKAVO_CEF_RENDERER_PATH") {
            return Ok(PathBuf::from(path));
        }

        let candidates = vec![
            "/Applications/Arkavo.app/Contents/MacOS/arkavo-cef-renderer",
            "/usr/local/bin/arkavo-cef-renderer",
            "./target/debug/arkavo-cef-renderer",
            "./target/release/arkavo-cef-renderer",
        ];

        for candidate in &candidates {
            let path = PathBuf::from(candidate);
            if path.exists() {
                return Ok(path);
            }
        }

        // Search in cargo build output directories
        let build_dirs = ["./target/debug/build", "./target/release/build"];

        for build_dir in &build_dirs {
            if let Ok(entries) = std::fs::read_dir(build_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with("arkavo-cef-"))
                        .unwrap_or(false)
                    {
                        let renderer_path = path.join(
                            "out/bin/arkavo-cef-renderer.app/Contents/MacOS/arkavo-cef-renderer",
                        );
                        if renderer_path.exists() {
                            return Ok(renderer_path);
                        }
                    }
                }
            }
        }

        Err(anyhow::anyhow!(
            "CEF renderer binary not found. Set ARKAVO_CEF_RENDERER_PATH or build the renderer."
        ))
    }

    fn commands(&mut self) -> &mut DOMCommandBuilder {
        self.renderer.commands()
    }

    pub fn set_event_callback<F>(&mut self, callback: F)
    where
        F: Fn(DOMEvent) + Send + Sync + 'static,
    {
        self.event_callback = Some(Arc::new(callback));
    }

    pub async fn poll_events(&mut self) -> Result<()> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl UiRenderer for CefRendererImpl {
    async fn render(&mut self, html: &str, css: &str, _js: &str) -> Result<()> {
        self.commands()
            .replace_inner_html("body", html)
            .await
            .map_err(|e| {
                error!("Failed to render HTML: {}", e);
                anyhow::anyhow!("CEF render error: {}", e)
            })?;

        let css_rule = format!("<style>{}</style>", css);
        self.commands()
            .replace_inner_html("head", &css_rule)
            .await
            .map_err(|e| {
                error!("Failed to render CSS: {}", e);
                anyhow::anyhow!("CEF CSS error: {}", e)
            })?;

        info!("CEF renderer: Rendered HTML and CSS successfully");
        Ok(())
    }

    async fn update_element(&mut self, selector: &str, html: &str) -> Result<()> {
        self.commands()
            .replace_inner_html(selector, html)
            .await
            .map_err(|e| anyhow::anyhow!("CEF update error: {}", e))?;
        Ok(())
    }

    async fn set_style(&mut self, selector: &str, property: &str, value: &str) -> Result<()> {
        self.commands()
            .set_style(selector, property, value)
            .await
            .map_err(|e| anyhow::anyhow!("CEF style error: {}", e))?;
        Ok(())
    }

    async fn add_event_listener(&mut self, selector: &str, event_type: &str) -> Result<()> {
        self.commands()
            .add_event_listener(selector, event_type)
            .await
            .map_err(|e| anyhow::anyhow!("CEF event listener error: {}", e))?;
        Ok(())
    }

    fn is_running(&self) -> bool {
        true
    }

    async fn shutdown(self: Box<Self>) -> Result<()> {
        info!("Shutting down CEF renderer");
        self.renderer
            .shutdown()
            .map_err(|e| anyhow::anyhow!("CEF shutdown error: {}", e))
    }
}
