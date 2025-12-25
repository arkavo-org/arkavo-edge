#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::significant_drop_in_scrutinee)]

pub mod a2a;
pub mod a2a_mcp_bridge;
pub mod agent_config;
pub mod agent_registry;
pub mod auth;
pub mod chat_session;
pub mod config;
pub mod config_transport;
pub mod discovery;
pub mod error;
pub mod file_transfer;
pub mod http;
pub mod mcp_registry;
#[cfg(feature = "mdns")]
pub mod mdns;
pub mod metrics;
pub mod metrics_subscription;
pub mod network;
pub mod oauth2;
pub mod openrpc;
pub mod push_notifications;
pub mod rate_limit;
pub mod registration;
pub mod security;
pub mod server;
pub mod session_persistence;
pub mod task_executor;
pub mod task_planner;
pub mod task_store;
pub mod transport;
pub mod types;
pub mod websocket;

pub use a2a_mcp_bridge::{A2aMcpBridge, McpToolRequest, McpToolResponse};
pub use config::{A2aConfig, A2aConfigBuilder, BufferConfig, ConfigManager};
pub use discovery::{DiscoveryConfig, DiscoveryMethod, DiscoveryService};
pub use error::{A2aError, Result};
pub use http::HttpTransport;
pub use mcp_registry::{McpConnectionTrait, McpRegistry};
#[cfg(feature = "mdns")]
pub use mdns::{MdnsError, MdnsServiceInfo};
pub use metrics::{MetricsCollector, RpcTimer};
pub use metrics_subscription::{
    MetricsApi, MetricsServiceConfig, MetricsSubscriptionServer, MetricsSubscriptionService,
};
pub use network::{NetworkError, get_service_ip};
pub use openrpc::{generate_openrpc_schema, openrpc_to_json};
pub use rate_limit::{
    IpRateLimiter, RateLimitConfig, RateLimitStatus, RateLimiter, spawn_cleanup_task,
};
pub use registration::{
    ChallengeRequest, ChallengeResponse, RegistrationService, RegistrationStatus, VerifyRequest,
    VerifyResponse,
};
pub use security::{AuthMethod, SecurityConfig, TlsSettings, TlsVersion};
pub use server::A2aServer;
pub use task_executor::{TaskEvent, TaskExecutor, TaskExecutorConfig};
pub use task_store::{SqliteTaskStore, Task, TaskStore};
pub use transport::{A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, TransportConfig};
pub use types::{
    AgentBroadcast, AgentQueryRequest, AgentQueryResponse, BroadcastType, DiscoverFeaturesDisclose,
    DiscoverFeaturesQuery, FeatureDisclosure, FeatureQuery, FeatureType,
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
