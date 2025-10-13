use anyhow::Result;
use async_trait::async_trait;

#[cfg(feature = "web-ui")]
pub mod web_renderer;

#[cfg(feature = "cef-ui")]
pub mod cef_renderer;

#[async_trait]
pub trait UiRenderer: Send + Sync {
    async fn render(&mut self, html: &str, css: &str, js: &str) -> Result<()>;

    async fn update_element(&mut self, selector: &str, html: &str) -> Result<()>;

    async fn set_style(&mut self, selector: &str, property: &str, value: &str) -> Result<()>;

    async fn add_event_listener(&mut self, selector: &str, event_type: &str) -> Result<()>;

    fn is_running(&self) -> bool;

    async fn shutdown(self: Box<Self>) -> Result<()>;
}

pub enum RendererType {
    #[cfg(feature = "web-ui")]
    Web,
    #[cfg(feature = "cef-ui")]
    Cef,
}

pub async fn create_renderer(renderer_type: RendererType) -> Result<Box<dyn UiRenderer>> {
    match renderer_type {
        #[cfg(feature = "web-ui")]
        RendererType::Web => {
            let renderer = web_renderer::WebRenderer::new().await?;
            Ok(Box::new(renderer))
        }
        #[cfg(feature = "cef-ui")]
        RendererType::Cef => {
            let renderer = cef_renderer::CefRendererImpl::new().await?;
            Ok(Box::new(renderer))
        }
    }
}

pub fn default_renderer_type() -> RendererType {
    #[cfg(feature = "cef-ui")]
    return RendererType::Cef;

    #[cfg(all(feature = "web-ui", not(feature = "cef-ui")))]
    return RendererType::Web;
}
