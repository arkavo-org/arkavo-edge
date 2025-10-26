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

        let mut renderer = CefRenderer::new(renderer_path).await?;

        // Wait for V8 context to initialize
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Initialize the page with complete HTML structure from Rust
        // Note: Event listeners will be attached by C++ OnLoadEnd handler
        let initial_html = r#"<html><head><style>
body { margin:0; padding:0; font-family:system-ui,-apple-system,sans-serif; height:100vh; display:flex; flex-direction:column; }
#content { flex:1; overflow:auto; padding:20px; background:linear-gradient(135deg,#667eea 0%,#764ba2 100%); color:#fff; }
#prompt-bar { position:fixed; bottom:0; left:0; right:0; background:#fff; padding:12px; box-shadow:0 -2px 8px rgba(0,0,0,0.1); display:flex; gap:8px; z-index:1000; }
#prompt-input { flex:1; padding:10px; border:1px solid #ddd; border-radius:4px; font-size:14px; }
#prompt-submit { padding:10px 20px; background:#667eea; color:#fff; border:none; border-radius:4px; cursor:pointer; font-size:14px; font-weight:600; }
#prompt-submit:hover { background:#5568d3; }
.welcome { text-align:center; padding-top:20vh; }
.welcome h1 { font-size:3em; margin:0; }
.welcome p { font-size:1.2em; opacity:0.9; margin-top:1em; }
</style></head><body>
<div id='content'>
<div class='welcome'>
<h1>Arkavo Edge</h1>
<p>Ready</p>
<p style='opacity:0.7;'>Enter your prompt below</p>
</div>
</div>
<div id='prompt-bar'>
<input type='text' id='prompt-input' placeholder='Enter your prompt...' />
<button id='prompt-submit'>Submit</button>
</div>
</body></html>"#;

        // Replace the entire document
        renderer
            .commands()
            .replace_inner_html("html", initial_html)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to set initial HTML: {}", e))?;

        info!("CEF renderer initialized with initial page structure");

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

    pub async fn try_recv_event(&mut self) -> Result<Option<arkavo_cef::DOMEvent>> {
        self.renderer
            .try_recv_event()
            .await
            .map_err(|e| anyhow::anyhow!("CEF event error: {}", e))
    }

    pub async fn try_recv_error(&mut self) -> Result<Option<arkavo_cef::DOMError>> {
        self.renderer
            .try_recv_error()
            .await
            .map_err(|e| anyhow::anyhow!("CEF error polling error: {}", e))
    }

    pub async fn try_recv_message(&mut self) -> Result<Option<arkavo_cef::uds::ReceivedMessage>> {
        self.renderer
            .commands()
            .try_recv_message()
            .await
            .map_err(|e| anyhow::anyhow!("CEF message polling error: {}", e))
    }
}

#[async_trait::async_trait]
impl UiRenderer for CefRendererImpl {
    async fn render(&mut self, html: &str, css: &str, _js: &str) -> Result<()> {
        println!(
            "[DEBUG] CefRendererImpl::render() called with {} bytes HTML, {} bytes CSS",
            html.len(),
            css.len()
        );

        let content_html = if css.is_empty() {
            html.to_string()
        } else {
            format!("<style>{}</style>{}", css, html)
        };

        println!(
            "[DEBUG] Calling replace_inner_html with {} bytes",
            content_html.len()
        );

        match self
            .commands()
            .replace_inner_html("#content", &content_html)
            .await
        {
            Ok(_) => {
                println!("[DEBUG] replace_inner_html succeeded");
                info!("CEF renderer: Rendered HTML and CSS successfully");
                Ok(())
            }
            Err(e) => {
                eprintln!("[ERROR] replace_inner_html failed: {}", e);
                error!("Failed to render HTML: {}", e);
                Err(anyhow::anyhow!("CEF render error: {}", e))
            }
        }
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
        // Note: Cannot check actual process status because trait requires &self
        // but CefRenderer::is_running() requires &mut self.
        // Shutdown is detected via explicit shutdown signal from CEF.
        true
    }

    async fn shutdown(self: Box<Self>) -> Result<()> {
        info!("Shutting down CEF renderer");
        self.renderer
            .shutdown()
            .map_err(|e| anyhow::anyhow!("CEF shutdown error: {}", e))
    }
}
