mod a2a_server;
mod conductor;
mod config_helpers;
mod episode_buffer;
mod event_loop;
mod gossip_transport;
mod handlers;
mod learning_bus;
mod mcp_bridge;
mod policy_cache;
mod rlm_bridge;
mod startup;
mod synthesis;
mod tool_memory;
mod tool_pattern_cache;
mod tool_pattern_observer;

pub use a2a_server::A2aServer;
pub use conductor::{execute_with_conductor, execute_with_conductor_and_learning};
pub use config_helpers::AgentMetadata;
pub use episode_buffer::{EpisodeBuffer, ToolObservation};
pub use event_loop::{start_event_processing_loop, start_lesson_application_loop};
pub use gossip_transport::{
    start_anti_entropy_loop, start_cleanup_loop, start_gossip_transport,
    start_lesson_propagation_loop,
};
pub use learning_bus::{BehaviorAdvice, LearningBus, LearningConfig, LearningEvent};
pub use mcp_bridge::McpBridgeTool;
pub use policy_cache::PolicyCache;
pub use rlm_bridge::{estimate_tokens, model_context_size, RlmBridge};
pub use startup::{AgentGoal, AgentPlan, GoalStatus, run_startup_planning_phase};
pub use tool_memory::{ToolMemory, ToolMemoryEntry};
pub use tool_pattern_cache::ToolPatternCache;
pub use tool_pattern_observer::ToolPatternObserver;

use crate::auth::AuthBackend;
use crate::mcp_registry::McpRegistry;
use crate::metrics::{MetricsCollector, RpcTimer};
use crate::rate_limit::RateLimiter;
use crate::task_executor::TaskExecutor;
use crate::task_store::TaskStore;
use crate::types::{
    AgentBroadcast, AgentConfigGetRequest, AgentConfigGetResponse, AgentConfigRestoreRequest,
    AgentConfigRestoreResponse, AgentConfigUpdateRequest, AgentConfigUpdateResponse,
    AgentConfigValidateRequest, AgentConfigValidateResponse, AgentDiscoverFilter,
    AgentQueryRequest, AgentQueryResponse, ChatOpenRequest, ChatRequest, ChatSession,
    DiscoverFeaturesDisclose, DiscoverFeaturesQuery, DiscoveredAgent, MessageSendRequest,
    MessageSendResponse, TaskCancelRequest, TaskCancelResponse, TaskCapability,
    TaskDeclareResponse, TaskGetRequest, TaskGetResponse, TaskResponse, UserMessage,
};
use arkavo_events::{Event, EventPayload, EventWriter};
use arkavo_hrm::{Conductor, store::InMemoryTaskStore};
use arkavo_llm::LlmClientAdapter;
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
    #[method(name = "registration.challenge")]
    async fn registration_challenge(
        &self,
        request: crate::registration::ChallengeRequest,
    ) -> RpcResult<crate::registration::ChallengeResponse>;

    /// Verify a registration challenge signature
    #[method(name = "registration.verify")]
    async fn registration_verify(
        &self,
        request: crate::registration::VerifyRequest,
    ) -> RpcResult<crate::registration::VerifyResponse>;

    /// Get registration status for a device
    #[method(name = "registration.status")]
    async fn registration_status(
        &self,
        device_id: String,
    ) -> RpcResult<crate::registration::RegistrationStatus>;

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
}

pub struct A2aRpcImpl {
    rate_limiter: Arc<RateLimiter>,
    metrics: Arc<MetricsCollector>,
    mcp_registry: Arc<McpRegistry>,
    agent_metadata: Arc<tokio::sync::RwLock<AgentMetadata>>,
    llm_adapter: Option<Arc<LlmClientAdapter>>,
    chat_sessions: Arc<crate::chat_session::ChatSessionManager>,
    task_store: Arc<dyn TaskStore>,
    task_executor: Arc<TaskExecutor>,
    event_writer: Option<Arc<EventWriter>>,
    session_id: String,
    event_sequence: Arc<tokio::sync::RwLock<u64>>,
    auth_backend: Arc<dyn AuthBackend>,
    registration_service: Arc<crate::registration::RegistrationService>,
    /// HRM Conductor for task orchestration
    conductor: Arc<Conductor<InMemoryTaskStore>>,
    /// Router for LLM calls during HRM task execution
    router: Option<Arc<arkavo_router::Router>>,
    /// Learning bus for gossip-based learning propagation
    learning_bus: Option<Arc<LearningBus>>,
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

    async fn registration_challenge(
        &self,
        request: crate::registration::ChallengeRequest,
    ) -> RpcResult<crate::registration::ChallengeResponse> {
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
        request: crate::registration::VerifyRequest,
    ) -> RpcResult<crate::registration::VerifyResponse> {
        handlers::registration::handle_registration_verify(
            &self.metrics,
            &self.rate_limiter,
            &self.registration_service,
            request,
        )
        .await
    }

    async fn registration_status(
        &self,
        device_id: String,
    ) -> RpcResult<crate::registration::RegistrationStatus> {
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
}
