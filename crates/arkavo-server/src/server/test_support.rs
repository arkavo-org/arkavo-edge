//! Fixtures shared by the server's unit tests.

use std::sync::Arc;

use arkavo_hrm::{Conductor, store::InMemoryTaskStore};
use arkavo_protocol::chat_session::ChatSessionManager;
use arkavo_protocol::config::BufferConfig;
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_protocol::metrics::MetricsCollector;
use arkavo_protocol::rate_limit::{RateLimitConfig, RateLimiter};
use arkavo_tasks::task_executor::{TaskExecutor, TaskExecutorConfig};
use arkavo_tasks::task_store::{SqliteTaskStore, TaskStore};
use tokio::sync::RwLock;

use super::A2aRpcImpl;
use super::config_helpers::{AgentMetadata, RoleSpecializationStore, session_auth_backend};
use super::handlers::specialization::UnconfiguredBundleDecryptor;
use super::tool_memory::ToolMemory;

/// The RPC implementation as `A2aServer` builds it for an agent that has no
/// router, learning bus or trust service, around `agent_metadata`.
///
/// Going through the real type keeps tests honest about the method surface:
/// the names they exercise are the ones the `#[rpc]` trait registers.
pub(super) async fn rpc_impl(agent_metadata: Arc<RwLock<AgentMetadata>>) -> A2aRpcImpl {
    let metrics = Arc::new(MetricsCollector::new(false));
    let task_store: Arc<dyn TaskStore> = Arc::new(
        SqliteTaskStore::new_in_memory()
            .await
            .expect("an in-memory task store needs no filesystem"),
    );
    let task_executor = Arc::new(TaskExecutor::with_metrics(
        task_store.clone(),
        TaskExecutorConfig::default(),
        metrics.clone(),
    ));

    A2aRpcImpl {
        rate_limiter: Arc::new(RateLimiter::new(RateLimitConfig::default())),
        metrics,
        mcp_registry: Arc::new(McpRegistry::new()),
        agent_metadata,
        role_specialization: Arc::new(RoleSpecializationStore::default()),
        bundle_decryptor: Arc::new(UnconfiguredBundleDecryptor),
        llm_adapter: None,
        chat_sessions: Arc::new(ChatSessionManager::with_config(
            None,
            None,
            None,
            3600,
            BufferConfig::default(),
        )),
        task_store,
        task_executor,
        event_writer: None,
        session_id: "test-session".to_string(),
        event_sequence: Arc::new(RwLock::new(0)),
        auth_backend: session_auth_backend(),
        registration_service: Arc::new(arkavo_agent::registration::RegistrationService::new()),
        conductor: Arc::new(Conductor::new(InMemoryTaskStore::new())),
        router: None,
        learning_bus: None,
        public_key: None,
        budget_manager: None,
        orchestrator_tick: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        model_hint: None,
        compute_budget: arkavo_budget::new_shared_compute_budget(),
        mesh_state: None,
        agent_memory: Arc::new(RwLock::new(ToolMemory::new(10))),
        trust_service: None,
        #[cfg(feature = "kas")]
        kas_handler: None,
        #[cfg(feature = "kas")]
        tdf_offer_store: Arc::new(super::handlers::tdf_share::TdfOfferStore::new()),
        agent_event_tx: Arc::new(tokio::sync::Mutex::new(None)),
        context_snapshot: Arc::new(RwLock::new(None)),
        #[cfg(feature = "iroh")]
        iroh_node: None,
    }
}
