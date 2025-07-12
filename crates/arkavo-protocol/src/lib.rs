pub mod a2a;
pub mod chat_session;
pub mod config;
pub mod discovery;
pub mod error;
pub mod http;
pub mod mcp;
pub mod mcp_registry;
pub mod metrics;
pub mod metrics_subscription;
pub mod openrpc;
pub mod rate_limit;
pub mod security;
pub mod server;
pub mod transport;
pub mod types;
pub mod websocket;

pub use config::{A2aConfig, A2aConfigBuilder, BufferConfig, ConfigManager};
pub use discovery::{DiscoveryConfig, DiscoveryMethod, DiscoveryService};
pub use error::{A2aError, Result};
pub use http::HttpTransport;
pub use mcp_registry::{McpConnectionTrait, McpRegistry};
pub use metrics::{MetricsCollector, RpcTimer};
pub use metrics_subscription::{
    MetricsApi, MetricsServiceConfig, MetricsSubscriptionServer, MetricsSubscriptionService,
};
pub use openrpc::{generate_openrpc_schema, openrpc_to_json};
pub use rate_limit::{
    IpRateLimiter, RateLimitConfig, RateLimitStatus, RateLimiter, spawn_cleanup_task,
};
pub use security::{AuthMethod, SecurityConfig, TlsSettings, TlsVersion};
pub use server::A2aServer;
pub use transport::{A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, TransportConfig};
pub use types::{
    DiscoverFeaturesDisclose, DiscoverFeaturesQuery, FeatureDisclosure, FeatureQuery, FeatureType,
};
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
