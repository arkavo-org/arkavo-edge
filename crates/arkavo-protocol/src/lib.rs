// Deep async call graphs through the router push trait solving past the default
// recursion limit; rustc 1.98 warns that the overflow will become a hard error.
// https://github.com/rust-lang/rust/issues/159228
#![recursion_limit = "256"]
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::significant_drop_in_scrutinee)]

//! # Arkavo Protocol
//!
//! Core A2A protocol implementation for the Arkavo agentic CLI tool.
//!
//! This crate contains the essential protocol types and communication primitives.
//! Related functionality has been moved to focused crates:
//!
//! - **`arkavo-security`** - Authentication, OAuth2, rate limiting, data classification
//! - **`arkavo-agent`** - Agent configuration, registry, discovery, registration
//! - **`arkavo-session`** - Chat sessions, WebSocket, HTTP, session persistence
//! - **`arkavo-tasks`** - Task execution, planning, and storage
//! - **`arkavo-server`** - A2A server implementation (JSON-RPC, handlers, HTTP)

// Core protocol modules (kept in this crate)
pub mod a2a;
pub mod a2a_mcp_bridge;
pub mod a2a_policy;
pub mod agent_config;
pub mod agent_registry;
pub mod agent_specialization;
pub mod auth;
pub mod chat_commands;
pub mod chat_session;
#[cfg(feature = "taint")]
pub mod classification_evidence;
pub mod config;
pub mod config_transport;
pub mod data_classification;
#[cfg(feature = "taint")]
pub mod derived_stamp;
pub mod discovery;
#[cfg(feature = "taint")]
pub mod egress_destination;
#[cfg(feature = "taint")]
pub mod egress_taint;
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
pub mod peer_manager;
#[cfg(feature = "taint")]
pub mod policy_join;
pub mod push_notifications;
pub mod rate_limit;
pub mod rate_limit_middleware;
pub mod registration;
pub mod security;
pub mod security_fixes;
#[cfg(feature = "taint")]
pub mod sequence_graph;
pub mod session_persistence;
#[cfg(feature = "taint")]
pub mod taint;
#[cfg(feature = "taint")]
pub mod taint_inference;
#[cfg(feature = "taint")]
mod taint_ledger;
#[cfg(feature = "taint")]
pub mod taint_tracker;
pub mod task_contract;
pub mod task_executor;
pub mod task_store;
#[cfg(feature = "taint")]
pub mod taxonomy;
pub mod transport;
pub mod types;
pub mod websocket;

// Re-export commonly used types from core modules
pub use a2a::{A2aClient, A2aClientError};
pub use a2a_mcp_bridge::{A2aMcpBridge, McpToolRequest, McpToolResponse};
pub use a2a_policy::{A2aAccess, A2aPolicy, A2aRule, PolicyMode};
pub use agent_specialization::{
    AgentPersona, AgentSpecializationBundle, BundleError, McpToolGrant, RoleContext,
    verify_dissemination_includes,
};
pub use auth::{AuthBackend, JwtAuthBackend, MultiAuthBackend, SessionAuth};
pub use chat_commands::{
    ChatSession, CommandResult, ContextMode, PendingContext, execute_command, parse_command,
};
pub use chat_session::ChatSessionManager;
#[cfg(feature = "taint")]
pub use classification_evidence::{
    ClassificationEvidence, Confidence, LabelFinding, TierOutcome, TierReport,
};
pub use config::{A2aConfig, A2aConfigBuilder, BufferConfig, ConfigManager};
pub use data_classification::{
    ClassifiedDatum, DataCategory, DatumType, DlpAction, DlpPolicy, SensitivityLevel,
};
#[cfg(feature = "taint")]
pub use derived_stamp::{
    DerivedAssertion, DerivedTag, DerivedWrap, SignedDerivedAssertion, plan_derived_wrap,
    sign_derived_assertion, verify_derived_assertion,
};
pub use discovery::{DiscoveryConfig, DiscoveryMethod, DiscoveryService};
#[cfg(feature = "taint")]
pub use egress_destination::{Destination, DestinationPolicy, extract_destinations};
#[cfg(feature = "taint")]
pub use egress_taint::{
    DenialReason, EgressDecision, EgressDisposition, EgressEvidence, EgressTaintGate, HoldReason,
    RequesterEntitlements,
};
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
#[cfg(feature = "taint")]
pub use policy_join::{PolicySet, UNKNOWN_VALUE};
pub use rate_limit::{IpRateLimiter, RateLimitConfig, RateLimiter, spawn_cleanup_task};
pub use rate_limit_middleware::{extract_client_ip, ip_rate_limit_middleware};
pub use registration::{
    ChallengeRequest, ChallengeResponse, RegistrationService, RegistrationStatus, VerifyRequest,
    VerifyResponse,
};
pub use security::{AuthMethod, SecurityConfig, TlsSettings, TlsVersion};
#[cfg(feature = "taint")]
pub use sequence_graph::{GraphError, NodeId, SequenceGraphBuilder, SequenceNode};
pub use session_persistence::SqliteSessionPersistence;
#[cfg(feature = "taint")]
pub use taint::{
    MAX_LABELS, MAX_PROVENANCE_HOPS, ProvenanceHop, SourceKind, TaintLabel, TaintSet, TaintSource,
    Transformation,
};
#[cfg(feature = "taint")]
pub use taint_inference::{ClassificationInferencer, RegexInferencer};
#[cfg(feature = "taint")]
pub use taint_tracker::{DEFAULT_FLOOR, DataTaintTracker, ModelCeilings};
pub use task_executor::{TaskEvent, TaskExecutor, TaskExecutorConfig};
pub use task_store::{SqliteTaskStore, Task, TaskStore};
#[cfg(feature = "taint")]
pub use taxonomy::{
    AttributeRequirement, LabelPolicy, TaxonomyMap, canonical_definition_fqn, canonical_value_fqn,
};
pub use transport::{A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, TransportConfig};
pub use types::{
    AgentBroadcast, AgentQueryRequest, AgentQueryResponse, BroadcastType, DiscoverFeaturesDisclose,
    DiscoverFeaturesQuery, FeatureDisclosure, FeatureQuery, FeatureType,
};
pub use websocket::WebSocketTransport;

// Re-export peer manager types
pub use peer_manager::{PeerManager, PeerManagerConfig, TransportType};

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
