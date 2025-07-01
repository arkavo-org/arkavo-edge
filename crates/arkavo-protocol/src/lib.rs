pub mod a2a;
pub mod config;
pub mod discovery;
pub mod error;
pub mod http;
pub mod mcp;
pub mod security;
pub mod transport;
pub mod websocket;

pub use config::{A2aConfig, A2aConfigBuilder, ConfigManager};
pub use discovery::{DiscoveryConfig, DiscoveryMethod, DiscoveryService};
pub use error::{A2aError, Result};
pub use http::HttpTransport;
pub use security::{AuthMethod, SecurityConfig, TlsSettings, TlsVersion};
pub use transport::{A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, TransportConfig};
pub use websocket::WebSocketTransport;

pub struct Client;

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    pub const fn new() -> Self {
        Self
    }

    pub fn send_message(
        &self,
        message: &str,
    ) -> std::result::Result<String, Box<dyn std::error::Error>> {
        Ok(format!("Response to: {message}"))
    }
}
