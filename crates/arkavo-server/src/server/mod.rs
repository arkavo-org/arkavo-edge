mod a2a_server;
mod anti_pattern;
mod autolearn_bridge;
mod conductor;
mod conductor_autoresearch;
mod conductor_evofabric;
mod conductor_planner;
mod conductor_tool_loop;
mod config_helpers;
mod consolidation;
mod curiosity;
mod episode_buffer;
mod event_loop;
mod gossip_transport;
mod handlers;
mod learning_bus;
mod learning_bus_gossip;
mod learning_bus_synthesis;
mod llm_intent_analyzer;
mod local_engine;
mod mcp_bridge;
mod policy_cache;
mod rlm_bridge;
mod startup;
mod synthesis;
mod tool_memory;
mod tool_pattern_cache;
mod tool_pattern_observer;
mod well_known;

pub use a2a_server::A2aServer;
pub use arkavo_autolearn::PainSignal;
pub use autolearn_bridge::AutoLearnBridge;
pub use conductor::{execute_with_conductor, execute_with_conductor_and_learning};
pub use config_helpers::AgentMetadata;
pub use episode_buffer::{EpisodeBuffer, ToolObservation};
pub use event_loop::{start_event_processing_loop, start_lesson_application_loop};
pub use gossip_transport::{
    start_advisor_broadcast_loop, start_anti_entropy_loop, start_cleanup_loop,
    start_gossip_transport, start_lesson_propagation_loop,
};
pub use learning_bus::{
    BehaviorAdvice, LearningBus, LearningBusStats, LearningConfig, LearningEvent,
    LearningPipelineReporter,
};
pub use local_engine::LocalEngine;
pub use mcp_bridge::McpBridgeTool;
pub use policy_cache::{PolicyCache, QualityTrend};
pub use rlm_bridge::{RlmBridge, estimate_tokens, model_context_size};
pub use startup::{AgentGoal, AgentPlan, GoalStatus, run_startup_planning_phase};
pub use tool_memory::{ToolMemory, ToolMemoryEntry};
pub use tool_pattern_cache::ToolPatternCache;
pub use tool_pattern_observer::ToolPatternObserver;
pub use well_known::WellKnownState;

use arkavo_agent::registration::{
    ChallengeRequest, ChallengeResponse, RegistrationService, RegistrationStatus, VerifyRequest,
    VerifyResponse,
};
use arkavo_events::{Event, EventPayload, EventWriter};
use arkavo_hrm::{Conductor, store::InMemoryTaskStore};
use arkavo_llm::LlmClientAdapter;
use arkavo_protocol::auth::AuthBackend;
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_protocol::metrics::{MetricsCollector, RpcTimer};
use arkavo_protocol::rate_limit::RateLimiter;
#[cfg(feature = "stub_handlers")]
use arkavo_protocol::types::TaskStatus;
use arkavo_protocol::types::{
    AgentBroadcast, AgentCapabilitiesGetResponse, AgentConfigGetRequest, AgentConfigGetResponse,
    AgentConfigRestoreRequest, AgentConfigRestoreResponse, AgentConfigUpdateRequest,
    AgentConfigUpdateResponse, AgentConfigValidateRequest, AgentConfigValidateResponse,
    AgentDiscoverFilter, AgentQueryRequest, AgentQueryResponse, AgentSpecializeRequest,
    AgentSpecializeResponse, ChatOpenRequest, ChatRequest, ChatSession, DiscoverFeaturesDisclose,
    DiscoverFeaturesQuery, DiscoveredAgent, KasPublicKeyRequest, KasPublicKeyResponse,
    KasRewrapRequest, KasRewrapResponse, MessageSendRequest, MessageSendResponse,
    TaskCancelRequest, TaskCancelResponse, TaskCapability, TaskDeclareResponse, TaskGetRequest,
    TaskGetResponse, TaskListRequest, TaskListResponse, TaskResponse, TdfOffersRequest,
    TdfOffersResponse, TdfShareRequest, TdfShareResponse, UserMessage,
};
use arkavo_tasks::task_executor::TaskExecutor;
use arkavo_tasks::task_store::TaskStore;
use async_trait::async_trait;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::{
    PendingSubscriptionSink,
    core::{RpcResult, SubscriptionResult},
    proc_macros::rpc,
};
use std::sync::Arc;

#[rpc(server)]
pub trait A2aRpc {
    #[method(name = "task_request")]
    async fn task_request(
        &self,
        agent_id: String,
        task_type: String,
        payload: Option<serde_json::Value>,
    ) -> RpcResult<TaskResponse>;

    #[method(name = "task_declare")]
    async fn task_declare(
        &self,
        agent_id: String,
        tasks: Vec<TaskCapability>,
    ) -> RpcResult<TaskDeclareResponse>;

    #[method(name = "agent_discover")]
    async fn agent_discover(
        &self,
        filter: Option<AgentDiscoverFilter>,
    ) -> RpcResult<Vec<DiscoveredAgent>>;

    #[method(name = "discover_features_query")]
    async fn discover_features_query(
        &self,
        query: Option<DiscoverFeaturesQuery>,
    ) -> RpcResult<DiscoverFeaturesDisclose>;

    #[method(name = "discover_features_disclose")]
    async fn discover_features_disclose(&self) -> RpcResult<DiscoverFeaturesDisclose>;

    #[method(name = "rpc.discover")]
    async fn rpc_discover(&self) -> RpcResult<serde_json::Value>;

    // A2A Protocol Methods

    /// Send a message synchronously
    #[method(name = "message/send")]
    async fn message_send(&self, request: MessageSendRequest) -> RpcResult<MessageSendResponse>;

    /// Get task status and result
    #[method(name = "tasks/get")]
    async fn tasks_get(&self, request: TaskGetRequest) -> RpcResult<TaskGetResponse>;

    /// Cancel a running task
    #[method(name = "tasks/cancel")]
    async fn tasks_cancel(&self, request: TaskCancelRequest) -> RpcResult<TaskCancelResponse>;

    /// List recent HRM tasks
    #[method(name = "tasks/list")]
    async fn tasks_list(&self, request: TaskListRequest) -> RpcResult<TaskListResponse>;

    /// Stream message updates
    #[subscription(name = "message/stream", unsubscribe = "message/stream/unsubscribe", item = MessageDelta)]
    async fn message_stream(&self, task_id: String) -> SubscriptionResult;

    /// Open a new chat session
    #[method(name = "chat_open")]
    async fn chat_open(&self, request: ChatOpenRequest) -> RpcResult<ChatSession>;

    /// Send a message within an existing chat session
    #[method(name = "chat_send")]
    async fn chat_send(&self, session_id: String, message: UserMessage) -> RpcResult<()>;

    /// Close a chat session
    #[method(name = "chat_close")]
    async fn chat_close(&self, session_id: String) -> RpcResult<()>;

    /// Subscribe to message deltas for a session
    #[subscription(name = "chat_stream", unsubscribe = "chat_stream_unsubscribe", item = MessageDelta)]
    async fn chat_stream(&self, session_id: String) -> SubscriptionResult;

    /// Acknowledge received message deltas (for back-pressure management)
    #[method(name = "chat_metrics_ack")]
    async fn chat_metrics_ack(&self, session_id: String, last_seq: u64) -> RpcResult<()>;

    /// Query another agent
    #[method(name = "agent_query")]
    async fn agent_query(&self, request: AgentQueryRequest) -> RpcResult<AgentQueryResponse>;

    /// Broadcast agent capabilities
    #[method(name = "agent_broadcast")]
    async fn agent_broadcast(&self, broadcast: AgentBroadcast) -> RpcResult<()>;

    /// Get agent configuration
    #[method(name = "agent.config.get")]
    async fn agent_config_get(
        &self,
        request: AgentConfigGetRequest,
    ) -> RpcResult<AgentConfigGetResponse>;

    /// Update agent configuration
    #[method(name = "agent.config.update")]
    async fn agent_config_update(
        &self,
        request: AgentConfigUpdateRequest,
    ) -> RpcResult<AgentConfigUpdateResponse>;

    /// Validate agent configuration
    #[method(name = "agent.config.validate")]
    async fn agent_config_validate(
        &self,
        request: AgentConfigValidateRequest,
    ) -> RpcResult<AgentConfigValidateResponse>;

    /// Restore agent configuration from backup
    #[method(name = "agent.config.restore")]
    async fn agent_config_restore(
        &self,
        request: AgentConfigRestoreRequest,
    ) -> RpcResult<AgentConfigRestoreResponse>;

    /// Legacy subscription method (to be deprecated)
    #[subscription(name = "chat_subscribe", unsubscribe = "chat_unsubscribe", item = MessageDelta)]
    async fn chat_subscribe(&self, request: ChatRequest) -> SubscriptionResult;

    /// Create a registration challenge
    /// Uses param_kind=map so iOS can send {"device_id": "..."} directly
    #[method(name = "registration.challenge", param_kind = map)]
    async fn registration_challenge(&self, device_id: String) -> RpcResult<ChallengeResponse>;

    /// Verify a registration challenge signature
    /// Uses param_kind=map so iOS can send {challenge_id, device_id, ...} directly
    #[method(name = "registration.verify", param_kind = map)]
    async fn registration_verify(
        &self,
        challenge_id: String,
        device_id: String,
        public_key: String,
        signature: String,
        delegation_jwt: Option<String>,
    ) -> RpcResult<VerifyResponse>;

    /// Get registration status for a device
    #[method(name = "registration.status")]
    async fn registration_status(&self, device_id: String) -> RpcResult<RegistrationStatus>;

    /// Handle incoming gossip message from peer
    #[method(name = "gossip/message")]
    async fn gossip_message(
        &self,
        message: arkavo_gossip::GossipMessage,
    ) -> RpcResult<Vec<arkavo_gossip::GossipMessage>>;

    /// Exchange public keys with peer for signature verification
    #[method(name = "agent/exchangeKeys")]
    async fn exchange_keys(&self, peer_id: String, public_key: String) -> RpcResult<String>;

    /// Check behavior policy for a sector based on learned lessons
    #[method(name = "learning/checkPolicy")]
    async fn check_policy(&self, sector_id: String) -> RpcResult<learning_bus::BehaviorAdvice>;

    /// Get Thompson Sampling learning state for UI dashboard
    #[method(name = "learning/status")]
    async fn learning_status(&self) -> RpcResult<serde_json::Value>;

    /// Get agent capabilities for orchestrator onboarding
    #[method(name = "agent.capabilities.get")]
    async fn agent_capabilities_get(&self) -> RpcResult<AgentCapabilitiesGetResponse>;

    /// Specialize agent with TDF-encrypted configuration
    #[method(name = "agent.specialize")]
    async fn agent_specialize(
        &self,
        request: AgentSpecializeRequest,
    ) -> RpcResult<AgentSpecializeResponse>;

    /// KAS rewrap - unwrap TDF key and rewrap for client
    #[method(name = "kas.rewrap")]
    async fn kas_rewrap(&self, request: KasRewrapRequest) -> RpcResult<KasRewrapResponse>;

    /// Get KAS public key for TDF encryption
    #[method(name = "kas.publicKey")]
    async fn kas_public_key(&self, request: KasPublicKeyRequest)
    -> RpcResult<KasPublicKeyResponse>;

    /// Share TDF-encrypted data with this agent via Iroh P2P
    #[method(name = "tdf.share")]
    async fn tdf_share(&self, request: TdfShareRequest) -> RpcResult<TdfShareResponse>;

    /// List pending TDF share offers
    #[method(name = "tdf.offers")]
    async fn tdf_offers(&self, request: TdfOffersRequest) -> RpcResult<TdfOffersResponse>;

    /// Get agent card (proxied from GET /.well-known/agent.json)
    #[method(name = "agent_card")]
    async fn agent_card(&self) -> RpcResult<serde_json::Value>;

    /// Health check (proxied from GET /health)
    #[method(name = "health")]
    async fn health(&self) -> RpcResult<serde_json::Value>;

    /// Get compute budget status for telemetry monitoring
    #[method(name = "budget.compute_status")]
    async fn budget_compute_status(&self) -> RpcResult<serde_json::Value>;

    /// Get process-level system metrics (RSS, CPU) for per-agent observability
    #[method(name = "system.metrics")]
    async fn system_metrics(&self) -> RpcResult<serde_json::Value>;

    /// Push-based metrics stream — replaces polling of system.metrics + budget.compute_status
    #[subscription(name = "system.metrics.subscribe", unsubscribe = "system.metrics.unsubscribe", item = serde_json::Value)]
    async fn system_metrics_subscribe(&self) -> SubscriptionResult;
}

pub struct A2aRpcImpl {
    pub(crate) rate_limiter: Arc<RateLimiter>,
    pub(crate) metrics: Arc<MetricsCollector>,
    pub(crate) mcp_registry: Arc<McpRegistry>,
    pub(crate) agent_metadata: Arc<tokio::sync::RwLock<AgentMetadata>>,
    pub(crate) llm_adapter: Option<Arc<LlmClientAdapter>>,
    pub(crate) chat_sessions: Arc<arkavo_protocol::chat_session::ChatSessionManager>,
    pub(crate) task_store: Arc<dyn TaskStore>,
    pub(crate) task_executor: Arc<TaskExecutor>,
    pub(crate) event_writer: Option<Arc<EventWriter>>,
    pub(crate) session_id: String,
    pub(crate) event_sequence: Arc<tokio::sync::RwLock<u64>>,
    pub(crate) auth_backend: Arc<dyn AuthBackend>,
    pub(crate) registration_service: Arc<RegistrationService>,
    /// HRM Conductor for task orchestration
    pub(crate) conductor: Arc<Conductor<InMemoryTaskStore>>,
    /// Router for LLM calls during HRM task execution
    pub(crate) router: Option<Arc<arkavo_router::Router>>,
    /// Learning bus for gossip-based learning propagation
    pub(crate) learning_bus: Option<Arc<LearningBus>>,
    /// Base64-encoded ECDSA P-256 public key for TDF encryption
    pub(crate) public_key: Option<String>,
    /// Budget manager for cost enforcement
    pub(crate) budget_manager: Option<Arc<arkavo_budget::BudgetManager>>,
    /// Orchestrator tick counter (shared with orchestrator loop)
    pub(crate) orchestrator_tick: Arc<std::sync::atomic::AtomicU64>,
    /// Model hint from AGENTS.md (bias for Thompson Sampling, not override)
    pub(crate) model_hint: Option<arkavo_router::ModelChoice>,
    /// Per-agent compute budget (specialists check before each tick)
    pub(crate) compute_budget: arkavo_budget::SharedComputeBudget,
    /// Mesh state for A2A delegation (send_task, list_agents, etc.)
    pub(crate) mesh_state: Option<Arc<arkavo_mcp_mesh::MeshToolsState>>,
    /// Shared tool memory for tracking recent actions across message/send calls
    pub(crate) agent_memory: Arc<tokio::sync::RwLock<ToolMemory>>,
    /// KAS A2A handler for TDF key operations
    #[cfg(feature = "kas")]
    pub(crate) kas_handler: Option<Arc<arkavo_tdf::KasA2aHandler>>,
    /// TDF share offer store for pending P2P offers
    #[cfg(feature = "kas")]
    pub(crate) tdf_offer_store: Arc<handlers::tdf_share::TdfOfferStore>,
    /// Shared Iroh P2P node for TDF blob transport
    #[cfg(feature = "iroh")]
    pub(crate) iroh_node: Option<Arc<arkavo_tdf_iroh::IrohNode>>,
}

#[async_trait]
impl A2aRpcServer for A2aRpcImpl {
    async fn task_request(
        &self,
        _agent_id: String,
        _task_type: String,
        _payload: Option<serde_json::Value>,
    ) -> RpcResult<TaskResponse> {
        let timer = RpcTimer::new("task_request".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        // Emit tool call event if event writer is configured
        if let Some(writer) = &self.event_writer {
            let agent_metadata = self.agent_metadata.read().await;
            let mut seq = self.event_sequence.write().await;
            let sequence = *seq;
            *seq += 1;

            let event = Event::new(
                self.session_id.clone(),
                sequence,
                agent_metadata.name.clone(),
                EventPayload::ToolCall {
                    tool_name: format!("task_{_task_type}"),
                    parameters: _payload.clone().unwrap_or(serde_json::Value::Null),
                    tool_call_id: Some(uuid::Uuid::new_v4().to_string()),
                },
            );
            let _ = writer.write(event).await;
        }

        #[cfg(feature = "stub_handlers")]
        {
            let response = TaskResponse {
                task_id: uuid::Uuid::new_v4(),
                status: TaskStatus::Submitted,
                data: _payload,
            };
            timer.success();
            Ok(response)
        }

        #[cfg(not(feature = "stub_handlers"))]
        {
            timer.error();
            Err(ErrorObjectOwned::owned(
                -32601,
                "Method not yet implemented",
                Some("task_request is not yet implemented".to_string()),
            ))
        }
    }

    async fn task_declare(
        &self,
        _agent_id: String,
        _tasks: Vec<TaskCapability>,
    ) -> RpcResult<TaskDeclareResponse> {
        let timer = RpcTimer::new("task_declare".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        #[cfg(feature = "stub_handlers")]
        {
            let response = TaskDeclareResponse {
                acknowledged: true,
                timestamp: chrono::Utc::now(),
            };
            timer.success();
            Ok(response)
        }

        #[cfg(not(feature = "stub_handlers"))]
        {
            timer.error();
            Err(ErrorObjectOwned::owned(
                -32601,
                "Method not yet implemented",
                Some("task_declare is not yet implemented".to_string()),
            ))
        }
    }

    async fn agent_discover(
        &self,
        filter: Option<AgentDiscoverFilter>,
    ) -> RpcResult<Vec<DiscoveredAgent>> {
        handlers::discovery::handle_agent_discover(
            &self.metrics,
            &self.rate_limiter,
            &self.mcp_registry,
            &self.agent_metadata,
            self.event_writer.as_ref(),
            &self.session_id,
            &self.event_sequence,
            filter,
        )
        .await
    }

    async fn discover_features_query(
        &self,
        query: Option<DiscoverFeaturesQuery>,
    ) -> RpcResult<DiscoverFeaturesDisclose> {
        handlers::discovery::handle_discover_features_query(
            &self.metrics,
            &self.rate_limiter,
            &self.mcp_registry,
            query,
        )
        .await
    }

    async fn discover_features_disclose(&self) -> RpcResult<DiscoverFeaturesDisclose> {
        self.discover_features_query(None).await
    }

    async fn rpc_discover(&self) -> RpcResult<serde_json::Value> {
        handlers::discovery::handle_rpc_discover(&self.metrics).await
    }

    // A2A Protocol Method Implementations

    async fn message_send(&self, request: MessageSendRequest) -> RpcResult<MessageSendResponse> {
        handlers::messaging::handle_message_send(
            &self.metrics,
            &self.rate_limiter,
            &self.task_executor,
            &self.task_store,
            &self.mcp_registry,
            &self.conductor,
            self.router.as_ref(),
            self.learning_bus.as_ref(),
            self.budget_manager.as_ref(),
            self.model_hint.clone(),
            &self.compute_budget,
            self.mesh_state.as_ref(),
            &self.agent_metadata,
            &self.agent_memory,
            #[cfg(feature = "iroh")]
            self.iroh_node.as_ref(),
            request,
        )
        .await
    }

    async fn tasks_get(&self, request: TaskGetRequest) -> RpcResult<TaskGetResponse> {
        handlers::tasks::handle_tasks_get(
            &self.metrics,
            &self.rate_limiter,
            &self.task_store,
            request,
        )
        .await
    }

    async fn tasks_cancel(&self, request: TaskCancelRequest) -> RpcResult<TaskCancelResponse> {
        handlers::tasks::handle_tasks_cancel(
            &self.metrics,
            &self.rate_limiter,
            &self.task_store,
            &self.task_executor,
            request,
        )
        .await
    }

    async fn tasks_list(&self, request: TaskListRequest) -> RpcResult<TaskListResponse> {
        handlers::tasks::handle_tasks_list(
            &self.metrics,
            &self.rate_limiter,
            &self.conductor,
            request,
        )
        .await
    }

    async fn chat_open(&self, request: ChatOpenRequest) -> RpcResult<ChatSession> {
        handlers::chat::handle_chat_open(
            &self.metrics,
            &self.rate_limiter,
            &self.auth_backend,
            &self.chat_sessions,
            request,
        )
        .await
    }

    async fn chat_send(&self, session_id: String, message: UserMessage) -> RpcResult<()> {
        handlers::chat::handle_chat_send(
            &self.metrics,
            &self.rate_limiter,
            &self.chat_sessions,
            self.router.as_ref(),
            session_id,
            message,
        )
        .await
    }

    async fn chat_close(&self, session_id: String) -> RpcResult<()> {
        handlers::chat::handle_chat_close(&self.metrics, &self.chat_sessions, session_id).await
    }

    async fn chat_metrics_ack(&self, session_id: String, last_seq: u64) -> RpcResult<()> {
        handlers::chat::handle_chat_metrics_ack(&self.metrics, &session_id, last_seq)
    }

    async fn agent_query(&self, request: AgentQueryRequest) -> RpcResult<AgentQueryResponse> {
        handlers::messaging::handle_agent_query(
            &self.metrics,
            &self.rate_limiter,
            &self.mcp_registry,
            &self.agent_metadata,
            request,
        )
        .await
    }

    async fn agent_broadcast(&self, broadcast: AgentBroadcast) -> RpcResult<()> {
        handlers::messaging::handle_agent_broadcast(
            &self.metrics,
            &self.rate_limiter,
            &self.mcp_registry,
            &self.agent_metadata,
            self.event_writer.as_ref(),
            &self.session_id,
            &self.event_sequence,
            broadcast,
        )
        .await
    }

    async fn agent_config_get(
        &self,
        request: AgentConfigGetRequest,
    ) -> RpcResult<AgentConfigGetResponse> {
        handlers::config::handle_config_get(&self.metrics, &self.rate_limiter, request).await
    }

    async fn agent_config_update(
        &self,
        request: AgentConfigUpdateRequest,
    ) -> RpcResult<AgentConfigUpdateResponse> {
        let agent_metadata = self.agent_metadata.clone();
        let has_llm_adapter = self.llm_adapter.is_some();
        let mcp_registry = self.mcp_registry.clone();

        handlers::config::handle_config_update(
            &self.metrics,
            &self.rate_limiter,
            request,
            |content| async move {
                handlers::config::reload_configuration(
                    &content,
                    &agent_metadata,
                    has_llm_adapter,
                    &mcp_registry,
                )
                .await
            },
        )
        .await
    }

    async fn agent_config_validate(
        &self,
        request: AgentConfigValidateRequest,
    ) -> RpcResult<AgentConfigValidateResponse> {
        handlers::config::handle_config_validate(&self.metrics, &self.rate_limiter, request).await
    }

    async fn agent_config_restore(
        &self,
        request: AgentConfigRestoreRequest,
    ) -> RpcResult<AgentConfigRestoreResponse> {
        handlers::config::handle_config_restore(&self.metrics, &self.rate_limiter, request).await
    }

    async fn message_stream(
        &self,
        sink: PendingSubscriptionSink,
        task_id: String,
    ) -> SubscriptionResult {
        handlers::streaming::handle_message_stream(&self.metrics, &self.rate_limiter, sink, task_id)
            .await
    }

    async fn chat_stream(
        &self,
        sink: PendingSubscriptionSink,
        session_id: String,
    ) -> SubscriptionResult {
        handlers::streaming::handle_chat_stream(
            &self.metrics,
            &self.rate_limiter,
            &self.chat_sessions,
            sink,
            session_id,
        )
        .await
    }

    async fn chat_subscribe(
        &self,
        sink: PendingSubscriptionSink,
        request: ChatRequest,
    ) -> SubscriptionResult {
        let agent_metadata = self.agent_metadata.read().await.clone();
        handlers::streaming::handle_chat_subscribe(
            &self.metrics,
            &self.rate_limiter,
            self.llm_adapter.clone(),
            agent_metadata,
            sink,
            request,
        )
        .await
    }

    async fn registration_challenge(&self, device_id: String) -> RpcResult<ChallengeResponse> {
        let request = ChallengeRequest { device_id };
        handlers::registration::handle_registration_challenge(
            &self.metrics,
            &self.rate_limiter,
            &self.registration_service,
            request,
        )
        .await
    }

    async fn registration_verify(
        &self,
        challenge_id: String,
        device_id: String,
        public_key: String,
        signature: String,
        delegation_jwt: Option<String>,
    ) -> RpcResult<VerifyResponse> {
        let request = VerifyRequest {
            challenge_id,
            device_id,
            public_key,
            signature,
            delegation_jwt,
        };
        handlers::registration::handle_registration_verify(
            &self.metrics,
            &self.rate_limiter,
            &self.registration_service,
            request,
        )
        .await
    }

    async fn registration_status(&self, device_id: String) -> RpcResult<RegistrationStatus> {
        handlers::registration::handle_registration_status(
            &self.metrics,
            &self.rate_limiter,
            &self.registration_service,
            device_id,
        )
        .await
    }

    async fn gossip_message(
        &self,
        message: arkavo_gossip::GossipMessage,
    ) -> RpcResult<Vec<arkavo_gossip::GossipMessage>> {
        let timer = RpcTimer::new("gossip_message".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        // Handle gossip message via LearningBus
        match &self.learning_bus {
            Some(bus) => {
                let responses = bus.handle_gossip(message).await;
                tracing::debug!("Gossip message processed, {} responses", responses.len());
                timer.success();
                Ok(responses)
            }
            None => {
                tracing::warn!("Gossip message received but LearningBus not configured");
                timer.error();
                Err(ErrorObjectOwned::owned(
                    -32603,
                    "LearningBus not configured",
                    None::<()>,
                ))
            }
        }
    }

    async fn exchange_keys(&self, peer_id: String, public_key: String) -> RpcResult<String> {
        let timer = RpcTimer::new("exchange_keys".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        // Parse incoming public key from base64
        let peer_key = match arkavo_crypto::AgentPublicKey::from_base64(&public_key) {
            Ok(key) => key,
            Err(e) => {
                timer.error();
                return Err(ErrorObjectOwned::owned(
                    -32602,
                    format!("Invalid public key: {e}"),
                    None::<()>,
                ));
            }
        };

        // Register peer's key in LearningBus
        match &self.learning_bus {
            Some(bus) => {
                bus.register_peer_key(peer_id.clone(), peer_key).await;

                // Return our public key
                let our_key = bus.keypair().public_key().to_base64();
                tracing::info!("Key exchange completed with peer: {}", peer_id);
                timer.success();
                Ok(our_key)
            }
            None => {
                tracing::warn!("Key exchange attempted but LearningBus not configured");
                timer.error();
                Err(ErrorObjectOwned::owned(
                    -32603,
                    "LearningBus not configured",
                    None::<()>,
                ))
            }
        }
    }

    async fn check_policy(&self, sector_id: String) -> RpcResult<learning_bus::BehaviorAdvice> {
        let timer = RpcTimer::new("check_policy".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        match &self.learning_bus {
            Some(bus) => {
                let advice = bus.check_behavior_policy(&sector_id).await;
                tracing::debug!("Policy check for sector {}: {:?}", sector_id, advice);
                timer.success();
                Ok(advice)
            }
            None => {
                tracing::debug!("Policy check: LearningBus not configured, returning default");
                timer.success();
                Ok(learning_bus::BehaviorAdvice::Default)
            }
        }
    }

    async fn learning_status(&self) -> RpcResult<serde_json::Value> {
        let timer = RpcTimer::new("learning_status".to_string(), self.metrics.clone());

        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        let router = match &self.router {
            Some(r) => r,
            None => {
                timer.success();
                return Ok(
                    serde_json::json!({ "agents": [], "selectedModel": null, "tickCount": 0 }),
                );
            }
        };

        let lm = router.model_learning();
        let all_stats = lm.get_all_stats().await;
        let mut agents = Vec::with_capacity(all_stats.len());

        for s in &all_stats {
            let cat_stats = lm.get_category_stats(&s.agent_id).await;
            let category_stats: Vec<serde_json::Value> = cat_stats
                .into_iter()
                .map(|(cat, alpha, beta_param, ev, obs)| {
                    serde_json::json!({
                        "category": cat,
                        "alpha": alpha,
                        "betaParam": beta_param,
                        "expectedValue": ev,
                        "observations": obs,
                    })
                })
                .collect();

            agents.push(serde_json::json!({
                "agentId": s.agent_id,
                "alpha": s.alpha,
                "betaParam": s.beta_param,
                "expectedValue": s.expected_value,
                "stdDev": s.std_dev,
                "totalObservations": s.total_observations,
                "successRate": s.success_rate,
                "probationary": s.probationary,
                "categoryStats": category_stats,
            }));
        }

        let tick = self
            .orchestrator_tick
            .load(std::sync::atomic::Ordering::Relaxed);

        // Include recent routing decisions for the UI dashboard
        let routing_history: Vec<serde_json::Value> = router
            .recent_decision_traces(20)
            .into_iter()
            .map(|t| {
                let was_exploration = matches!(
                    t.selection_reason,
                    arkavo_router::learning::SelectionReason::Probationary
                );
                serde_json::json!({
                    "taskId": t.trace_id.to_string(),
                    "selectedAgent": t.selected_model,
                    "wasExploration": was_exploration,
                    "category": t.task_category.as_str(),
                    "qualityScore": null,
                    "timestamp": t.timestamp.to_rfc3339(),
                })
            })
            .collect();

        let optimal_configs: Vec<serde_json::Value> = router
            .optimal_configs
            .snapshot()
            .into_iter()
            .map(|(name, oc)| {
                serde_json::json!({
                    "model": name,
                    "temperature": oc.temperature,
                    "topP": oc.top_p,
                    "thinkingMode": format!("{:?}", oc.thinking_mode),
                    "confidence": oc.confidence,
                })
            })
            .collect();

        timer.success();
        Ok(serde_json::json!({
            "agents": agents,
            "selectedModel": router.last_routed_model(),
            "tickCount": tick,
            "routingHistory": routing_history,
            "optimalConfigs": optimal_configs,
        }))
    }

    async fn agent_capabilities_get(&self) -> RpcResult<AgentCapabilitiesGetResponse> {
        handlers::discovery::handle_agent_capabilities_get(
            &self.metrics,
            &self.rate_limiter,
            &self.mcp_registry,
            &self.agent_metadata,
            self.public_key.as_deref(),
        )
        .await
    }

    async fn agent_specialize(
        &self,
        request: AgentSpecializeRequest,
    ) -> RpcResult<AgentSpecializeResponse> {
        handlers::specialization::handle_agent_specialize(
            &self.metrics,
            &self.rate_limiter,
            &self.mcp_registry,
            &self.agent_metadata,
            request,
        )
        .await
    }

    async fn kas_rewrap(&self, request: KasRewrapRequest) -> RpcResult<KasRewrapResponse> {
        #[cfg(feature = "kas")]
        {
            // Extract caller DID from authenticated context
            // For now, use a placeholder - real impl would get this from auth
            let caller_did = "did:key:z6MkUnknown";

            handlers::kas::handle_kas_rewrap(
                &self.metrics,
                &self.rate_limiter,
                self.kas_handler.as_ref(),
                request,
                caller_did,
            )
            .await
        }

        #[cfg(not(feature = "kas"))]
        {
            let _ = request;
            Err(ErrorObjectOwned::owned(
                -32603,
                "KAS capability not available",
                Some("Build with --features kas to enable KAS capability".to_string()),
            ))
        }
    }

    async fn kas_public_key(
        &self,
        request: KasPublicKeyRequest,
    ) -> RpcResult<KasPublicKeyResponse> {
        #[cfg(feature = "kas")]
        {
            handlers::kas::handle_kas_public_key(
                &self.metrics,
                &self.rate_limiter,
                self.kas_handler.as_ref(),
                request,
            )
            .await
        }

        #[cfg(not(feature = "kas"))]
        {
            let _ = request;
            Err(ErrorObjectOwned::owned(
                -32603,
                "KAS capability not available",
                Some("Build with --features kas to enable KAS capability".to_string()),
            ))
        }
    }

    async fn tdf_share(&self, request: TdfShareRequest) -> RpcResult<TdfShareResponse> {
        #[cfg(feature = "kas")]
        {
            handlers::tdf_share::handle_tdf_share(
                &self.metrics,
                &self.rate_limiter,
                &self.tdf_offer_store,
                request,
            )
            .await
        }

        #[cfg(not(feature = "kas"))]
        {
            let _ = request;
            Err(ErrorObjectOwned::owned(
                -32603,
                "TDF share not available",
                Some("Build with --features kas to enable TDF sharing".to_string()),
            ))
        }
    }

    async fn tdf_offers(&self, request: TdfOffersRequest) -> RpcResult<TdfOffersResponse> {
        #[cfg(feature = "kas")]
        {
            handlers::tdf_share::handle_tdf_offers(
                &self.metrics,
                &self.rate_limiter,
                &self.tdf_offer_store,
                request,
            )
            .await
        }

        #[cfg(not(feature = "kas"))]
        {
            let _ = request;
            Err(ErrorObjectOwned::owned(
                -32603,
                "TDF offers not available",
                Some("Build with --features kas to enable TDF sharing".to_string()),
            ))
        }
    }

    async fn agent_card(&self) -> RpcResult<serde_json::Value> {
        #[allow(clippy::needless_update)]
        let state = well_known::WellKnownState {
            agent_metadata: self.agent_metadata.clone(),
            mcp_registry: self.mcp_registry.clone(),
            rpc_port: 0,
            rate_limiter: Arc::new(arkavo_protocol::IpRateLimiter::new(
                arkavo_protocol::RateLimitConfig::default(),
            )),
            #[cfg(feature = "kas")]
            kas_enabled: true,
        };
        let card = well_known::build_agent_card(&state).await;
        serde_json::to_value(card).map_err(|e| {
            ErrorObjectOwned::owned(-32603, format!("Serialization error: {e}"), None::<()>)
        })
    }

    async fn health(&self) -> RpcResult<serde_json::Value> {
        Ok(serde_json::json!({"status": "ok"}))
    }

    async fn budget_compute_status(&self) -> RpcResult<serde_json::Value> {
        let budget = self.compute_budget.read().await;
        let snapshot = budget.snapshot();
        let agent_meta = self.agent_metadata.read().await;
        let agent_id = agent_meta.name.clone();
        drop(agent_meta);
        drop(budget);

        serde_json::to_value(serde_json::json!({
            "agent_id": agent_id,
            "compute_budget": snapshot,
        }))
        .map_err(|e| {
            ErrorObjectOwned::owned(-32603, format!("Serialization error: {e}"), None::<()>)
        })
    }

    async fn system_metrics(&self) -> RpcResult<serde_json::Value> {
        use sysinfo::{Pid, System};
        let pid = std::process::id();
        let mut sys = System::new();
        sys.refresh_memory();
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            true,
            sysinfo::ProcessRefreshKind::nothing()
                .with_memory()
                .with_cpu(),
        );
        let (rss_mb, cpu_percent) = if let Some(proc) = sys.process(Pid::from_u32(pid)) {
            (
                proc.memory() as f64 / (1024.0 * 1024.0),
                proc.cpu_usage() as f64,
            )
        } else {
            (0.0, 0.0)
        };
        let total_ram_mb = sys.total_memory() as f64 / (1024.0 * 1024.0);
        let available_ram_mb = sys.available_memory() as f64 / (1024.0 * 1024.0);
        let agent_meta = self.agent_metadata.read().await;
        let agent_id = agent_meta.name.clone();
        drop(agent_meta);

        let timing = arkavo_observability::subsystem_timing::global_timing().snapshot();
        let subsystem_timing = if timing.is_empty() {
            None
        } else {
            Some(timing)
        };

        #[cfg(feature = "iroh")]
        let iroh_active = self.iroh_node.is_some();
        #[cfg(not(feature = "iroh"))]
        let iroh_active = false;

        Ok(serde_json::json!({
            "agent_id": agent_id,
            "rss_mb": rss_mb,
            "cpu_percent": cpu_percent,
            "pid": pid,
            "total_ram_mb": total_ram_mb,
            "available_ram_mb": available_ram_mb,
            "subsystem_timing": subsystem_timing,
            "iroh_active": iroh_active,
        }))
    }

    async fn system_metrics_subscribe(&self, sink: PendingSubscriptionSink) -> SubscriptionResult {
        let sink = match sink.accept().await {
            Ok(s) => s,
            Err(_) => return Ok(()),
        };

        let agent_metadata = self.agent_metadata.clone();
        let compute_budget = self.compute_budget.clone();
        let agent_memory = self.agent_memory.clone();
        let learning_bus = self.learning_bus.clone();
        let router = self.router.clone();
        #[cfg(feature = "iroh")]
        let iroh_node = self.iroh_node.clone();

        tokio::spawn(async move {
            loop {
                use sysinfo::{Pid, System};
                let pid = std::process::id();
                let mut sys = System::new();
                sys.refresh_memory();
                sys.refresh_processes_specifics(
                    sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
                    true,
                    sysinfo::ProcessRefreshKind::nothing()
                        .with_memory()
                        .with_cpu(),
                );
                let (rss_mb, cpu_percent) = if let Some(proc) = sys.process(Pid::from_u32(pid)) {
                    (
                        proc.memory() as f64 / (1024.0 * 1024.0),
                        proc.cpu_usage() as f64,
                    )
                } else {
                    (0.0, 0.0)
                };

                let agent_id = agent_metadata.read().await.name.clone();

                let timing = arkavo_observability::subsystem_timing::global_timing().snapshot();
                let subsystem_timing = if timing.is_empty() {
                    None
                } else {
                    Some(timing)
                };

                let budget_snapshot = {
                    let b = compute_budget.read().await;
                    serde_json::to_value(b.snapshot()).ok()
                };

                #[cfg(feature = "iroh")]
                let iroh_active = iroh_node.is_some();
                #[cfg(not(feature = "iroh"))]
                let iroh_active = false;

                // Context topology: tool memory
                let tool_memory_snap = agent_memory.read().await.topology_snapshot();

                // Context topology: gossip stats
                let gossip_snap = if let Some(bus) = &learning_bus {
                    let stats = bus.stats().await;
                    serde_json::json!({
                        "eventsReceived": stats.events_received,
                        "episodesSynthesized": stats.episodes_synthesized,
                        "lessonsStored": stats.lessons_stored,
                        "gossipPeers": stats.gossip_peers,
                        "lastEventSecsAgo": stats.last_event_secs_ago,
                    })
                } else {
                    serde_json::json!(null)
                };

                // Context topology: anti-patterns
                let anti_patterns_snap: Vec<serde_json::Value> = if let Some(bus) = &learning_bus {
                    let cache = bus.policy_cache().read().await;
                    let mut patterns = Vec::new();
                    if let Some(r) = &router {
                        let lm = r.model_learning();
                        let all_stats = lm.get_all_stats().await;
                        for s in &all_stats {
                            for ap in cache.anti_patterns.get_for_model(&s.agent_id) {
                                patterns.push(serde_json::json!({
                                    "model": ap.model,
                                    "category": ap.category,
                                    "failureSignature": ap.failure_signature,
                                    "failureCount": ap.failure_count,
                                    "decayedWeight": ap.decayed_weight(),
                                    "lastSeen": ap.last_seen.to_rfc3339(),
                                }));
                            }
                        }
                    }
                    patterns
                } else {
                    vec![]
                };

                // Context topology: decision traces
                let traces_snap: Vec<serde_json::Value> = if let Some(r) = &router {
                    r.recent_decision_traces(10)
                        .into_iter()
                        .map(|t| {
                            serde_json::json!({
                                "traceId": t.trace_id.to_string(),
                                "agentId": &agent_id,
                                "taskCategory": t.task_category.as_str(),
                                "selectedModel": t.selected_model,
                                "selectionReason": match &t.selection_reason {
                                    arkavo_router::learning::SelectionReason::ThompsonSampling { score, .. } =>
                                        format!("Thompson({score:.2})"),
                                    arkavo_router::learning::SelectionReason::BudgetConstrained => "BudgetConstrained".to_string(),
                                    arkavo_router::learning::SelectionReason::SingleFeasible => "SingleFeasible".to_string(),
                                    arkavo_router::learning::SelectionReason::Probationary => "Probationary".to_string(),
                                    arkavo_router::learning::SelectionReason::Fallback => "Fallback".to_string(),
                                },
                                "budgetUsagePct": t.budget_usage_pct,
                                "feasibleCount": t.feasible_models.len(),
                                "timestamp": t.timestamp.to_rfc3339(),
                            })
                        })
                        .collect()
                } else {
                    vec![]
                };

                let metrics = serde_json::json!({
                    "agent_id": agent_id,
                    "rss_mb": rss_mb,
                    "cpu_percent": cpu_percent,
                    "pid": pid,
                    "total_ram_mb": sys.total_memory() as f64 / (1024.0 * 1024.0),
                    "available_ram_mb": sys.available_memory() as f64 / (1024.0 * 1024.0),
                    "subsystem_timing": subsystem_timing,
                    "iroh_active": iroh_active,
                    "compute_budget": budget_snapshot,
                    "context_topology": {
                        "toolMemory": tool_memory_snap,
                        "gossip": gossip_snap,
                        "antiPatterns": anti_patterns_snap,
                        "decisionTraces": traces_snap,
                        "contextUtilizationPct": conductor::PEAK_CONTEXT_UTILIZATION_BP
                            .load(std::sync::atomic::Ordering::Relaxed) as f64 / 100.0,
                    },
                });

                if let Ok(msg) = serde_json::value::to_raw_value(&metrics)
                    && sink.send(msg).await.is_err()
                {
                    break; // Client disconnected
                }

                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });

        Ok(())
    }
}
