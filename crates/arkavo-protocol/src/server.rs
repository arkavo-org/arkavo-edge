use crate::auth::{AuthBackend, NoOpAuthBackend};
use crate::config::{BufferConfig, ServerConfig};
use crate::error::{A2aError, Result};
use crate::mcp_registry::McpRegistry;
use crate::metrics::{MetricsCollector, RpcTimer};
use crate::openrpc;
use crate::rate_limit::RateLimiter;
use crate::task_executor::{TaskExecutor, TaskExecutorConfig};
use crate::task_store::{SqliteTaskStore, TaskStore};
use crate::types::{
    AgentBroadcast, AgentConfigGetRequest, AgentConfigGetResponse, AgentConfigRestoreRequest,
    AgentConfigRestoreResponse, AgentConfigUpdateRequest, AgentConfigUpdateResponse,
    AgentConfigValidateRequest, AgentConfigValidateResponse, AgentDiscoverFilter,
    AgentQueryRequest, AgentQueryResponse, BroadcastType, ChatOpenRequest, ChatRequest,
    ChatSession, ConfigError, DiscoverFeaturesDisclose, DiscoverFeaturesQuery, DiscoveredAgent,
    FeatureDisclosure, FeatureType, Message, MessageDelta, MessageDeltaContent, MessageSendRequest,
    MessageSendResponse, TaskCancelRequest, TaskCancelResponse, TaskCapability,
    TaskDeclareResponse, TaskGetRequest, TaskGetResponse, TaskResponse, TaskStatus, UserMessage,
};
use arkavo_events::{Event, EventPayload, EventWriter, EventWriterConfig};
use arkavo_llm::{DeltaType, LlmClient, LlmClientAdapter, StreamLlmModel};
use async_trait::async_trait;
use futures::StreamExt;
use jsonrpsee::server::{ServerBuilder, ServerHandle};
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::{
    PendingSubscriptionSink, SubscriptionMessage,
    core::{RpcResult, SubscriptionResult},
    proc_macros::rpc,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{error, info, warn};

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
}

#[derive(Default, Clone)]
struct AgentMetadata {
    name: String,
    purpose: String,
    model: String,
    endpoint: String,
    api_keys: std::collections::HashMap<String, String>,
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
        _filter: Option<AgentDiscoverFilter>,
    ) -> RpcResult<Vec<DiscoveredAgent>> {
        let timer = RpcTimer::new("agent_discover".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        // Emit agent discover event
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
                    tool_name: "agent_discover".to_string(),
                    parameters: serde_json::json!({
                        "filter": _filter
                    }),
                    tool_call_id: Some(uuid::Uuid::new_v4().to_string()),
                },
            );
            let _ = writer.write(event).await;
        }

        // Get MCP tools and server status
        let mcp_tools = match self.mcp_registry.list_all_tools().await {
            Ok(tools) => tools.into_iter().map(|t| t.name).collect::<Vec<String>>(),
            Err(_) => Vec::new(),
        };

        let mcp_servers = self.mcp_registry.get_server_status().await;

        // Build metadata with MCP information
        let (name, purpose, model, endpoint) = {
            let metadata = self.agent_metadata.read().await;
            (
                metadata.name.clone(),
                metadata.purpose.clone(),
                metadata.model.clone(),
                metadata.endpoint.clone(),
            )
        };

        let metadata_json = serde_json::json!({
            "name": name,
            "purpose": purpose,
            "model": model,
            "mcp_tools": mcp_tools,
            "mcp_servers": mcp_servers,
        });

        // Define task types based on agent capabilities
        let task_types = vec![
            "chat".to_string(),
            "code_editing".to_string(),
            "test_execution".to_string(),
            "diff_preview".to_string(),
        ];

        // Add task capabilities to metadata
        let metadata_with_tasks = {
            let mut meta = metadata_json;
            meta["task_capabilities"] = serde_json::json!([
                {
                    "type": "chat",
                    "constraints": {
                        "max_context_length": 8192,
                        "supports_streaming": true,
                    }
                },
                {
                    "type": "code_editing",
                    "constraints": {
                        "languages": ["rust", "python", "javascript", "typescript"],
                        "supports_refactoring": true,
                    }
                },
                {
                    "type": "test_execution",
                    "constraints": {
                        "frameworks": ["cargo", "pytest", "jest"],
                    }
                },
                {
                    "type": "diff_preview",
                    "constraints": {
                        "formats": ["unified", "split"],
                    }
                }
            ]);
            meta
        };

        let agent = DiscoveredAgent {
            agent_id: uuid::Uuid::new_v4(), // Generate a unique ID for the agent
            endpoint,
            tasks: Some(task_types),
            metadata: Some(metadata_with_tasks),
        };

        timer.success();
        Ok(vec![agent])
    }

    async fn discover_features_query(
        &self,
        query: Option<DiscoverFeaturesQuery>,
    ) -> RpcResult<DiscoverFeaturesDisclose> {
        let timer = RpcTimer::new("discover_features_query".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        // Build disclosures based on query
        let mut disclosures = Vec::new();

        // Add base protocol support
        disclosures.push(FeatureDisclosure {
            feature_type: FeatureType::Protocol,
            id: "https://didcomm.org/discover-features/2.0".to_string(),
            roles: Some(vec!["requester".to_string(), "responder".to_string()]),
        });

        // Add A2A protocol support
        disclosures.push(FeatureDisclosure {
            feature_type: FeatureType::Protocol,
            id: "https://arkavo.org/a2a/1.0".to_string(),
            roles: Some(vec!["agent".to_string()]),
        });

        // Add MCP tools if available
        match self.mcp_registry.list_all_tools().await {
            Ok(tools) => {
                for tool in tools {
                    disclosures.push(FeatureDisclosure {
                        feature_type: FeatureType::McpTool,
                        id: tool.name,
                        roles: None,
                    });
                }
            }
            Err(_) => {
                // Ignore errors, just don't include tools
            }
        }

        // Add MCP servers
        let mcp_servers = self.mcp_registry.get_server_status().await;
        for (server_name, status) in mcp_servers {
            disclosures.push(FeatureDisclosure {
                feature_type: FeatureType::McpServer,
                id: format!("{server_name} ({status})"),
                roles: None,
            });
        }

        // Filter based on query if provided
        if let Some(query) = query
            && let Some(queries) = query.queries
        {
            disclosures.retain(|disclosure| {
                queries.iter().any(|q| {
                    if q.feature_type as i32 != disclosure.feature_type as i32 {
                        return false;
                    }
                    if let Some(pattern) = &q.match_pattern {
                        // Simple wildcard matching
                        if pattern.contains('*') {
                            let prefix = pattern.trim_end_matches('*');
                            disclosure.id.starts_with(prefix)
                        } else {
                            disclosure.id == *pattern
                        }
                    } else {
                        true
                    }
                })
            });
        }

        timer.success();
        Ok(DiscoverFeaturesDisclose { disclosures })
    }

    async fn discover_features_disclose(&self) -> RpcResult<DiscoverFeaturesDisclose> {
        // Proactive disclosure - return all features without filtering
        self.discover_features_query(None).await
    }

    async fn rpc_discover(&self) -> RpcResult<serde_json::Value> {
        let timer = RpcTimer::new("rpc.discover".to_string(), self.metrics.clone());
        let schema = openrpc::generate_openrpc_schema();

        match serde_json::to_value(schema) {
            Ok(value) => {
                timer.success();
                Ok(value)
            }
            Err(e) => {
                timer.error();
                Err(ErrorObjectOwned::owned(
                    -32603,
                    "Failed to serialize OpenRPC schema",
                    Some(e.to_string()),
                ))
            }
        }
    }

    // A2A Protocol Method Implementations

    async fn message_send(&self, request: MessageSendRequest) -> RpcResult<MessageSendResponse> {
        let timer = RpcTimer::new("message/send".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        // Submit the task using our task executor
        match self.task_executor.submit_task(request.message).await {
            Ok(task_id) => {
                let response = MessageSendResponse {
                    task_id: task_id.to_string(),
                    status: TaskStatus::Submitted,
                    response: None, // Async processing, no immediate response
                };
                timer.success();
                Ok(response)
            }
            Err(e) => {
                timer.error();
                Err(ErrorObjectOwned::owned(
                    -32603,
                    "Failed to submit task",
                    Some(format!("Error: {e}")),
                ))
            }
        }
    }

    async fn tasks_get(&self, request: TaskGetRequest) -> RpcResult<TaskGetResponse> {
        let timer = RpcTimer::new("tasks/get".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        // Parse task ID
        let task_id = match uuid::Uuid::parse_str(&request.task_id) {
            Ok(id) => id,
            Err(_) => {
                timer.error();
                return Err(ErrorObjectOwned::owned(
                    -32602,
                    "Invalid task ID",
                    Some("Task ID must be a valid UUID".to_string()),
                ));
            }
        };

        // Get task from store
        match self.task_store.get_task(&task_id).await {
            Ok(Some(task)) => {
                // Get task result if completed
                let result = if task.status == TaskStatus::Completed {
                    self.task_store
                        .get_task_result(&task_id)
                        .await
                        .ok()
                        .flatten()
                        .and_then(|v| serde_json::from_value::<Message>(v).ok())
                } else {
                    task.result
                };

                let response = TaskGetResponse {
                    task_id: task.id.to_string(),
                    status: task.status,
                    result,
                    error: task.error,
                    progress: task.progress,
                };
                timer.success();
                Ok(response)
            }
            Ok(None) => {
                timer.error();
                Err(ErrorObjectOwned::owned(
                    -32602,
                    "Task not found",
                    Some(format!("No task found with ID: {}", request.task_id)),
                ))
            }
            Err(e) => {
                timer.error();
                Err(ErrorObjectOwned::owned(
                    -32603,
                    "Failed to retrieve task",
                    Some(format!("Error: {e}")),
                ))
            }
        }
    }

    async fn tasks_cancel(&self, request: TaskCancelRequest) -> RpcResult<TaskCancelResponse> {
        let timer = RpcTimer::new("tasks/cancel".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        // Parse task ID
        let task_id = match uuid::Uuid::parse_str(&request.task_id) {
            Ok(id) => id,
            Err(_) => {
                timer.error();
                return Err(ErrorObjectOwned::owned(
                    -32602,
                    "Invalid task ID",
                    Some("Task ID must be a valid UUID".to_string()),
                ));
            }
        };

        // Try to cancel the task
        match self
            .task_executor
            .update_task_status(&task_id, TaskStatus::Canceled)
            .await
        {
            Ok(()) => {
                let response = TaskCancelResponse {
                    success: true,
                    status: TaskStatus::Canceled,
                    message: request
                        .reason
                        .or_else(|| Some("Task cancelled successfully".to_string())),
                };
                timer.success();
                Ok(response)
            }
            Err(e) => {
                // Get the current task status
                let current_status = self
                    .task_store
                    .get_task(&task_id)
                    .await
                    .ok()
                    .flatten()
                    .map(|t| t.status)
                    .unwrap_or(TaskStatus::Failed);

                let response = TaskCancelResponse {
                    success: false,
                    status: current_status,
                    message: Some(format!("Failed to cancel task: {e}")),
                };
                timer.success(); // Still a valid response
                Ok(response)
            }
        }
    }

    async fn message_stream(
        &self,
        sink: PendingSubscriptionSink,
        _task_id: String,
    ) -> SubscriptionResult {
        let timer = RpcTimer::new("message/stream".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(_e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Ok(());
        }

        // Accept the subscription
        let _sink = match sink.accept().await {
            Ok(sink) => sink,
            Err(_) => {
                timer.error();
                return Ok(());
            }
        };

        #[cfg(feature = "stub_handlers")]
        {
            // Send a few mock updates
            tokio::spawn(async move {
                // Send initial status
                let delta = MessageDelta {
                    session_id: _task_id.clone(),
                    message_id: uuid::Uuid::new_v4().to_string(),
                    sequence: 0,
                    delta: MessageDeltaContent::Text {
                        text: "Processing task...".to_string(),
                    },
                    timestamp: chrono::Utc::now(),
                };

                if let Ok(msg) = SubscriptionMessage::from_json(&delta) {
                    let _ = _sink.send(msg).await;
                }

                // Simulate some processing time
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

                // Send completion
                let delta = MessageDelta {
                    session_id: _task_id,
                    message_id: uuid::Uuid::new_v4().to_string(),
                    sequence: 1,
                    delta: MessageDeltaContent::StreamEnd {
                        reason: crate::types::StreamEndReason::Complete,
                    },
                    timestamp: chrono::Utc::now(),
                };

                if let Ok(msg) = SubscriptionMessage::from_json(&delta) {
                    let _ = _sink.send(msg).await;
                }
            });
        }

        timer.success();
        Ok(())
    }

    async fn chat_open(&self, _request: ChatOpenRequest) -> RpcResult<ChatSession> {
        let timer = RpcTimer::new("chat_open".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        // Validate JWT token if provided
        let auth = if let Some(token) = _request.token {
            match self.auth_backend.validate_token(&token).await {
                Ok(auth) => Some(auth),
                Err(e) => {
                    timer.error();
                    return Err(ErrorObjectOwned::owned(
                        -32004,
                        format!("Authentication failed: {e}"),
                        None::<()>,
                    ));
                }
            }
        } else {
            None
        };

        // Create a new chat session with authentication
        let session = self.chat_sessions.create_session(auth).await;

        timer.success();
        Ok(session)
    }

    async fn chat_send(&self, session_id: String, message: UserMessage) -> RpcResult<()> {
        let timer = RpcTimer::new("chat_send".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        // Send message to the session
        match self.chat_sessions.send_message(&session_id, message).await {
            Ok(()) => {
                timer.success();
                Ok(())
            }
            Err(e) => {
                timer.error();
                Err(ErrorObjectOwned::owned(
                    e.to_json_rpc_code(),
                    "Failed to send message",
                    Some(e.to_string()),
                ))
            }
        }
    }

    async fn chat_close(&self, session_id: String) -> RpcResult<()> {
        let timer = RpcTimer::new("chat_close".to_string(), self.metrics.clone());

        // Close the session
        match self.chat_sessions.close_session(&session_id).await {
            Ok(()) => {
                timer.success();
                Ok(())
            }
            Err(e) => {
                timer.error();
                Err(ErrorObjectOwned::owned(
                    e.to_json_rpc_code(),
                    "Failed to close session",
                    Some(e.to_string()),
                ))
            }
        }
    }

    async fn chat_stream(
        &self,
        sink: PendingSubscriptionSink,
        session_id: String,
    ) -> SubscriptionResult {
        let timer = RpcTimer::new("chat_stream".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(_e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Ok(());
        }

        // Accept the subscription
        let sink = match sink.accept().await {
            Ok(sink) => sink,
            Err(_) => {
                timer.error();
                return Ok(());
            }
        };

        // Get delta stream for this session
        if let Some(mut delta_rx) = self.chat_sessions.get_delta_stream(&session_id).await {
            // Spawn a task to forward deltas to the subscription
            tokio::spawn(async move {
                while let Some(delta) = delta_rx.recv().await {
                    if let Ok(msg) = SubscriptionMessage::from_json(&delta)
                        && sink.send(msg).await.is_err()
                    {
                        break; // Client disconnected
                    }
                }
            });

            timer.success();
            Ok(())
        } else {
            timer.error();
            // Session not found - subscription will be closed
            Ok(())
        }
    }

    async fn chat_subscribe(
        &self,
        sink: PendingSubscriptionSink,
        request: ChatRequest,
    ) -> SubscriptionResult {
        let timer = RpcTimer::new("chat_subscribe".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(_e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Ok(());
        }

        // Accept the subscription
        let sink = match sink.accept().await {
            Ok(sink) => sink,
            Err(_) => {
                timer.error();
                return Ok(());
            }
        };

        // Generate a unique message ID for this conversation
        let message_id = uuid::Uuid::new_v4().to_string();
        let trace_id = uuid::Uuid::new_v4().to_string();

        // Clone LLM adapter if available
        let llm_adapter = self.llm_adapter.clone();
        let agent_metadata = self.agent_metadata.read().await.clone();

        // Spawn a task to handle the streaming response
        tokio::spawn(async move {
            if let Some(adapter) = llm_adapter {
                // Create chat request
                let chat_request = arkavo_llm::ChatRequest::new(request.message);

                // Start streaming from LLM
                match adapter.stream_chat(chat_request, trace_id).await {
                    Ok((_stream_id, mut delta_stream)) => {
                        while let Some(delta_result) = delta_stream.next().await {
                            match delta_result {
                                Ok(stream_delta) => {
                                    // Convert StreamDelta to MessageDelta
                                    let message_delta = match stream_delta.delta {
                                        DeltaType::Text { content } => MessageDelta {
                                            session_id: request
                                                .session_id
                                                .clone()
                                                .unwrap_or_default(),
                                            message_id: message_id.clone(),
                                            sequence: 0,
                                            delta: MessageDeltaContent::Text { text: content },
                                            timestamp: stream_delta.timestamp,
                                        },
                                        DeltaType::ToolCall {
                                            id,
                                            name,
                                            arguments,
                                        } => MessageDelta {
                                            session_id: request
                                                .session_id
                                                .clone()
                                                .unwrap_or_default(),
                                            message_id: message_id.clone(),
                                            sequence: 0,
                                            delta: MessageDeltaContent::ToolCall {
                                                tool_call_id: id,
                                                name: Some(name),
                                                args_json_fragment: arguments
                                                    .map(|v| v.to_string())
                                                    .unwrap_or_else(|| "{}".to_string()),
                                                done: false, // Will be set to true on stream end
                                            },
                                            timestamp: stream_delta.timestamp,
                                        },
                                        DeltaType::Error(err) => {
                                            error!(
                                                code = err.code,
                                                message = err.message,
                                                "Stream error during chat delta processing"
                                            );
                                            continue;
                                        }
                                        DeltaType::StreamEnd { reason: _ } => {
                                            break;
                                        }
                                    };

                                    // Send the delta using the subscription sink
                                    if let Ok(msg) = SubscriptionMessage::from_json(&message_delta)
                                        && sink.send(msg).await.is_err()
                                    {
                                        break; // Client disconnected
                                    }
                                }
                                Err(e) => {
                                    error!(error = %e, "Delta stream error");
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "Failed to start LLM stream");
                        let error_delta = MessageDelta {
                            session_id: request.session_id.clone().unwrap_or_default(),
                            message_id: message_id.clone(),
                            sequence: 0,
                            delta: MessageDeltaContent::Text {
                                text: format!("Error: Failed to start LLM stream - {e}"),
                            },
                            timestamp: chrono::Utc::now(),
                        };

                        if let Ok(msg) = SubscriptionMessage::from_json(&error_delta) {
                            let _ = sink.send(msg).await;
                        }
                    }
                }
            } else {
                // No LLM configured - send error message
                let error_delta = MessageDelta {
                    session_id: request.session_id.clone().unwrap_or_default(),
                    message_id: message_id.clone(),
                    sequence: 0,
                    delta: MessageDeltaContent::Text {
                        text: format!(
                            "Error: No LLM configured for agent '{}'. Model: '{}'",
                            agent_metadata.name, agent_metadata.model
                        ),
                    },
                    timestamp: chrono::Utc::now(),
                };

                if let Ok(msg) = SubscriptionMessage::from_json(&error_delta) {
                    let _ = sink.send(msg).await;
                }
            }
        });

        timer.success();
        Ok(())
    }

    async fn agent_query(&self, request: AgentQueryRequest) -> RpcResult<AgentQueryResponse> {
        let timer = RpcTimer::new("agent_query".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        let metadata = self.agent_metadata.read().await;
        let agent_id = metadata.name.clone();
        drop(metadata);

        // If target agent is specified, check if it's registered
        if let Some(ref target_id) = request.to_agent_id {
            let agents = self.mcp_registry.list_agents().await;
            if !agents.iter().any(|a| &a.identity.id == target_id) {
                timer.error();
                return Err(jsonrpsee::types::error::ErrorObject::owned(
                    -32001,
                    format!("Target agent not found: {target_id}"),
                    None::<()>,
                ));
            }
        }

        // Process query based on domain if specified
        let mut response_text = format!("Processing query: {}", request.query);
        let mut confidence = 0.5;
        let mut evidence = None;

        if let Some(ref domain) = request.domain {
            match domain.as_str() {
                "code" => {
                    response_text = format!("Code analysis for: {}", request.query);
                    confidence = 0.7;
                    evidence = Some(serde_json::json!({
                        "domain": "code",
                        "analyzed": true
                    }));
                }
                "data" => {
                    response_text = format!("Data query result for: {}", request.query);
                    confidence = 0.8;
                    evidence = Some(serde_json::json!({
                        "domain": "data",
                        "records_examined": 100
                    }));
                }
                "general" => {
                    response_text = format!("General response to: {}", request.query);
                    confidence = 0.6;
                }
                _ => {
                    response_text = format!("Unknown domain {}: {}", domain, request.query);
                    confidence = 0.3;
                }
            }
        }

        // LLM integration is handled through the LlmAdapter trait
        // configured during server initialization

        let response = AgentQueryResponse {
            from_agent_id: agent_id,
            response: response_text,
            confidence,
            domain: request.domain,
            evidence,
        };

        info!(
            "Agent query processed: from={}, to={:?}, confidence={}",
            request.from_agent_id, request.to_agent_id, confidence
        );

        timer.success();
        Ok(response)
    }

    async fn agent_broadcast(&self, broadcast: AgentBroadcast) -> RpcResult<()> {
        let timer = RpcTimer::new("agent_broadcast".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        // Update agent metadata if this is a capability broadcast
        if matches!(broadcast.broadcast_type, BroadcastType::Capability)
            && let Some(ref capabilities) = broadcast.capabilities
        {
            // Store capabilities in metadata for future reference
            for capability in capabilities {
                info!(
                    "Agent {} announced capability: {} - {:?}",
                    broadcast.agent_id, capability.name, capability.description
                );
            }
        }

        // Handle different broadcast types
        match broadcast.broadcast_type {
            BroadcastType::Status => {
                info!(
                    "Agent {} status broadcast: {:?}",
                    broadcast.agent_id, broadcast.status
                );

                // Update registry with agent status
                if let Some(ref status) = broadcast.status {
                    self.mcp_registry
                        .update_agent_status(&broadcast.agent_id, status.clone())
                        .await;
                }
            }
            BroadcastType::Capability => {
                info!(
                    "Agent {} capability broadcast: {} capabilities",
                    broadcast.agent_id,
                    broadcast.capabilities.as_ref().map_or(0, |c| c.len())
                );
            }
            BroadcastType::Availability => {
                info!(
                    "Agent {} availability broadcast: accepting_tasks={:?}",
                    broadcast.agent_id, broadcast.accepting_tasks
                );
            }
            BroadcastType::Shutdown => {
                warn!("Agent {} is shutting down", broadcast.agent_id);

                // Remove agent from registry
                self.mcp_registry
                    .unregister_agent(&broadcast.agent_id)
                    .await;
            }
            BroadcastType::Custom(ref custom_type) => {
                info!(
                    "Agent {} custom broadcast type: {}",
                    broadcast.agent_id, custom_type
                );
            }
        }

        // Store broadcast for future reference
        if let Some(ref event_writer) = self.event_writer {
            // Use ReasoningStep as a general-purpose event type for now
            let event_payload = EventPayload::ReasoningStep {
                step_type: "agent_broadcast".to_string(),
                description: format!(
                    "Agent {} broadcast: {:?}",
                    broadcast.agent_id, broadcast.broadcast_type
                ),
                metadata: Some(serde_json::json!({
                    "agent_id": broadcast.agent_id,
                    "broadcast_type": broadcast.broadcast_type,
                    "capabilities": broadcast.capabilities,
                    "status": broadcast.status,
                    "metadata": broadcast.metadata
                })),
            };

            let agent_metadata = self.agent_metadata.read().await;
            let event = Event::new(
                self.session_id.clone(),
                *self.event_sequence.read().await,
                agent_metadata.name.clone(),
                event_payload,
            );
            drop(agent_metadata);

            *self.event_sequence.write().await += 1;

            if let Err(e) = event_writer.write(event).await {
                warn!("Failed to write broadcast event: {}", e);
            }
        }

        timer.success();
        Ok(())
    }

    async fn agent_config_get(
        &self,
        request: AgentConfigGetRequest,
    ) -> RpcResult<AgentConfigGetResponse> {
        let timer = RpcTimer::new("agent_config_get".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        // Read AGENTS.md file
        let config_path = std::path::Path::new("AGENTS.md");
        let content = match tokio::fs::read_to_string(&config_path).await {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                timer.error();
                return Err(ErrorObjectOwned::owned(
                    -32002,
                    "Configuration file not found",
                    Some(serde_json::json!({
                        "error": ConfigError::AgentOffline
                    })),
                ));
            }
            Err(e) => {
                timer.error();
                return Err(ErrorObjectOwned::owned(
                    -32603,
                    format!("Failed to read configuration: {e}"),
                    Some(serde_json::json!({
                        "error": ConfigError::ReadOnlyFilesystem
                    })),
                ));
            }
        };

        // Calculate version hash
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let version = format!("{:x}", hasher.finalize());

        // List backups if requested
        let backups = if request.include_backups {
            let backup_dir = std::path::Path::new(".agents.md.backup");
            let mut backup_list = Vec::new();

            if backup_dir.exists()
                && let Ok(mut entries) = tokio::fs::read_dir(backup_dir).await
            {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    if let Ok(metadata) = entry.metadata().await
                        && metadata.is_file()
                    {
                        let filename = entry.file_name().to_string_lossy().to_string();
                        if filename.starts_with("AGENTS.md.") && filename.ends_with(".backup") {
                            // Extract timestamp from filename
                            let timestamp_str = filename
                                .strip_prefix("AGENTS.md.")
                                .and_then(|s| s.strip_suffix(".backup"))
                                .unwrap_or("");

                            let timestamp = chrono::NaiveDateTime::parse_from_str(
                                timestamp_str,
                                "%Y-%m-%d-%H%M%S",
                            )
                            .ok()
                            .map(|dt| {
                                chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                                    dt,
                                    chrono::Utc,
                                )
                            })
                            .unwrap_or_else(chrono::Utc::now);

                            // Calculate backup version
                            if let Ok(backup_content) =
                                tokio::fs::read_to_string(entry.path()).await
                            {
                                let mut backup_hasher = Sha256::new();
                                backup_hasher.update(backup_content.as_bytes());
                                let backup_version = format!("{:x}", backup_hasher.finalize());

                                backup_list.push(crate::types::ConfigBackup {
                                    filename,
                                    timestamp,
                                    size: metadata.len(),
                                    version: backup_version,
                                });
                            }
                        }
                    }
                }
            }

            // Sort by timestamp, newest first
            backup_list.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
            Some(backup_list)
        } else {
            None
        };

        // Check if file is writable
        let writable = config_path
            .metadata()
            .map(|m| !m.permissions().readonly())
            .unwrap_or(false);

        timer.success();
        Ok(AgentConfigGetResponse {
            content,
            version,
            backups,
            writable,
        })
    }

    async fn agent_config_update(
        &self,
        request: AgentConfigUpdateRequest,
    ) -> RpcResult<AgentConfigUpdateResponse> {
        let timer = RpcTimer::new("agent_config_update".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        let config_path = std::path::Path::new("AGENTS.md");

        // Check expected version for optimistic locking
        if let Some(expected_version) = &request.expected_version
            && let Ok(current_content) = tokio::fs::read_to_string(&config_path).await
        {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(current_content.as_bytes());
            let current_version = format!("{:x}", hasher.finalize());

            if &current_version != expected_version {
                timer.error();
                return Ok(AgentConfigUpdateResponse {
                    success: false,
                    new_version: None,
                    backup_path: None,
                    error: Some(ConfigError::Conflict { current_version }),
                    reload_required: false,
                });
            }
        }

        // Validate the new configuration
        if let Err(validation_error) = validate_agent_config(&request.content) {
            timer.error();
            return Ok(AgentConfigUpdateResponse {
                success: false,
                new_version: None,
                backup_path: None,
                error: Some(ConfigError::ValidationFailed {
                    details: validation_error,
                }),
                reload_required: false,
            });
        }

        // Create backup if requested
        let backup_path = if request.create_backup {
            // Create backup directory if it doesn't exist
            let backup_dir = std::path::Path::new(".agents.md.backup");
            if !backup_dir.exists()
                && let Err(e) = tokio::fs::create_dir_all(backup_dir).await
            {
                warn!("Failed to create backup directory: {}", e);
            }

            // Create backup with timestamp
            let timestamp = chrono::Utc::now().format("%Y-%m-%d-%H%M%S");
            let backup_filename = format!("AGENTS.md.{timestamp}.backup");
            let backup_path = backup_dir.join(&backup_filename);

            // Copy current file to backup
            if config_path.exists() {
                if let Err(e) = tokio::fs::copy(&config_path, &backup_path).await {
                    warn!("Failed to create backup: {}", e);
                    None
                } else {
                    // Keep only last 10 backups
                    cleanup_old_backups(backup_dir, 10).await;
                    Some(backup_path.to_string_lossy().to_string())
                }
            } else {
                None
            }
        } else {
            None
        };

        // Save last-known-good before update
        let last_good_path = std::path::Path::new("AGENTS.md.last-known-good");
        if config_path.exists() {
            let _ = tokio::fs::copy(&config_path, &last_good_path).await;
        }

        // Write new configuration atomically
        let temp_path = std::path::Path::new("AGENTS.md.tmp");
        match tokio::fs::write(&temp_path, &request.content).await {
            Ok(_) => {
                // Atomic rename
                if let Err(_e) = tokio::fs::rename(&temp_path, &config_path).await {
                    timer.error();
                    return Ok(AgentConfigUpdateResponse {
                        success: false,
                        new_version: None,
                        backup_path,
                        error: Some(ConfigError::ReadOnlyFilesystem),
                        reload_required: false,
                    });
                }
            }
            Err(_e) => {
                timer.error();
                let _ = tokio::fs::remove_file(&temp_path).await;
                return Ok(AgentConfigUpdateResponse {
                    success: false,
                    new_version: None,
                    backup_path,
                    error: Some(ConfigError::ReadOnlyFilesystem),
                    reload_required: false,
                });
            }
        }

        // Calculate new version
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(request.content.as_bytes());
        let new_version = format!("{:x}", hasher.finalize());

        // Reload configuration automatically
        let reload_success = match self.reload_configuration(&request.content).await {
            Ok(_) => {
                info!("Configuration updated and reloaded successfully");
                true
            }
            Err(e) => {
                warn!(
                    "Configuration saved but reload failed: {}. Agent restart may be required.",
                    e
                );
                false
            }
        };

        timer.success();
        Ok(AgentConfigUpdateResponse {
            success: true,
            new_version: Some(new_version),
            backup_path,
            error: None,
            reload_required: !reload_success, // Only require reload if auto-reload failed
        })
    }

    async fn agent_config_validate(
        &self,
        request: AgentConfigValidateRequest,
    ) -> RpcResult<AgentConfigValidateResponse> {
        let timer = RpcTimer::new("agent_config_validate".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // Parse and validate the configuration
        match parse_agents_config(&request.content) {
            Ok(agents) => {
                if agents.is_empty() {
                    errors.push("No agent configurations found".to_string());
                } else {
                    for agent in agents {
                        // Validate required fields
                        if agent.name.is_empty() {
                            errors.push("Agent name is required".to_string());
                        }
                        if agent.purpose.is_empty() {
                            warnings.push(format!("Agent '{}' has no purpose defined", agent.name));
                        }
                        if agent.model.is_empty() {
                            errors.push(format!("Agent '{}' requires a model", agent.name));
                        }
                        if agent.listen.is_empty() {
                            errors
                                .push(format!("Agent '{}' requires a listen address", agent.name));
                        }

                        // Validate listen address format
                        if !agent.listen.is_empty()
                            && agent.listen.parse::<std::net::SocketAddr>().is_err()
                        {
                            errors.push(format!(
                                "Agent '{}' has invalid listen address: {}",
                                agent.name, agent.listen
                            ));
                        }

                        // Validate model format
                        if !agent.model.is_empty() && !agent.model.contains("://") {
                            warnings.push(format!(
                                "Agent '{}' model should use protocol format (e.g., ollama://...)",
                                agent.name
                            ));
                        }
                    }
                }
            }
            Err(e) => {
                errors.push(format!("Configuration parse error: {e}"));
            }
        }

        timer.success();
        Ok(AgentConfigValidateResponse {
            valid: errors.is_empty(),
            errors,
            warnings,
        })
    }

    async fn agent_config_restore(
        &self,
        request: AgentConfigRestoreRequest,
    ) -> RpcResult<AgentConfigRestoreResponse> {
        let timer = RpcTimer::new("agent_config_restore".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        let backup_dir = std::path::Path::new(".agents.md.backup");
        let backup_path = backup_dir.join(&request.backup_filename);

        // Check if backup exists
        if !backup_path.exists() {
            timer.error();
            return Ok(AgentConfigRestoreResponse {
                success: false,
                new_version: None,
                error: Some(ConfigError::ValidationFailed {
                    details: format!("Backup file not found: {}", request.backup_filename),
                }),
            });
        }

        // Read backup content
        let backup_content = match tokio::fs::read_to_string(&backup_path).await {
            Ok(content) => content,
            Err(_e) => {
                timer.error();
                return Ok(AgentConfigRestoreResponse {
                    success: false,
                    new_version: None,
                    error: Some(ConfigError::ReadOnlyFilesystem),
                });
            }
        };

        // Validate backup content
        if let Err(validation_error) = validate_agent_config(&backup_content) {
            timer.error();
            return Ok(AgentConfigRestoreResponse {
                success: false,
                new_version: None,
                error: Some(ConfigError::ValidationFailed {
                    details: format!("Backup validation failed: {validation_error}"),
                }),
            });
        }

        // Create a backup of current config before restoring
        let config_path = std::path::Path::new("AGENTS.md");
        if config_path.exists() {
            let timestamp = chrono::Utc::now().format("%Y-%m-%d-%H%M%S");
            let pre_restore_backup =
                backup_dir.join(format!("AGENTS.md.{timestamp}.pre-restore.backup"));
            let _ = tokio::fs::copy(&config_path, &pre_restore_backup).await;
        }

        // Restore the backup
        match tokio::fs::write(&config_path, &backup_content).await {
            Ok(_) => {
                // Calculate new version
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(backup_content.as_bytes());
                let new_version = format!("{:x}", hasher.finalize());

                info!(
                    "Configuration restored from backup: {}",
                    request.backup_filename
                );

                timer.success();
                Ok(AgentConfigRestoreResponse {
                    success: true,
                    new_version: Some(new_version),
                    error: None,
                })
            }
            Err(_e) => {
                timer.error();
                Ok(AgentConfigRestoreResponse {
                    success: false,
                    new_version: None,
                    error: Some(ConfigError::ReadOnlyFilesystem),
                })
            }
        }
    }
}

impl A2aRpcImpl {
    /// Hot-reload configuration from new content
    async fn reload_configuration(&self, content: &str) -> Result<()> {
        use crate::agent_config::parse_agents_config;

        // Validate configuration before applying
        if content.trim().is_empty() {
            return Err(A2aError::Configuration(
                "Configuration file is empty".to_string(),
            ));
        }

        // Parse the new configuration
        let agents = parse_agents_config(content)
            .map_err(|e| A2aError::Configuration(format!("Failed to parse configuration: {e}")))?;

        if agents.is_empty() {
            return Err(A2aError::Configuration(
                "No agent configurations found".to_string(),
            ));
        }

        // Find our agent's configuration
        let agent_metadata = self.agent_metadata.read().await;
        let our_agent_name = &agent_metadata.name;

        let new_config = agents
            .iter()
            .find(|a| a.name == *our_agent_name)
            .ok_or_else(|| {
                A2aError::Configuration(format!(
                    "Agent '{our_agent_name}' not found in new configuration"
                ))
            })?;

        // Validate required fields
        if new_config.purpose.is_empty() {
            return Err(A2aError::Configuration(
                "Agent purpose cannot be empty".to_string(),
            ));
        }
        if new_config.model.is_empty() {
            return Err(A2aError::Configuration(
                "Agent model cannot be empty".to_string(),
            ));
        }

        // Store model before releasing lock
        let old_model = agent_metadata.model.clone();
        drop(agent_metadata); // Release read lock

        // Update metadata
        {
            let mut metadata = self.agent_metadata.write().await;
            metadata.purpose.clone_from(&new_config.purpose);
            // Note: listen address is stored in endpoint field
            metadata.endpoint.clone_from(&new_config.listen);

            // Update API keys in metadata and environment
            metadata.api_keys.clone_from(&new_config.api_keys);
            for (key, value) in &new_config.api_keys {
                // SAFETY: We control the key and value from our config file
                unsafe {
                    std::env::set_var(key, value);
                }
            }

            info!("Updated agent metadata for '{}'", metadata.name);
            info!("  Purpose: {}", metadata.purpose);
            info!("  Endpoint: {}", metadata.endpoint);
            info!("  API keys updated: {}", metadata.api_keys.len());
        }

        // If model changed, recreate LLM adapter
        if new_config.model != old_model && self.llm_adapter.is_some() {
            // Note: In a real implementation, we'd need to recreate the adapter
            // This would require access to the LLM client creation logic
            warn!(
                "Model change detected from '{}' to '{}', but LLM adapter recreation not yet implemented",
                old_model, new_config.model
            );

            // Update the model in metadata for now
            let mut metadata = self.agent_metadata.write().await;
            metadata.model.clone_from(&new_config.model);
        }

        // Handle MCP server changes
        if !new_config.mcp_servers.is_empty() {
            info!("MCP server configuration changed, clearing existing connections");

            // Clear existing MCP connections
            self.mcp_registry.clear_connections().await;

            // Log the MCP servers that need to be restarted
            for mcp_server in &new_config.mcp_servers {
                info!("MCP server '{}' needs restart:", mcp_server.name);
                if let Some(cmd) = &mcp_server.command {
                    info!("  Command: {} {:?}", cmd, mcp_server.args);
                } else if let Some(url) = &mcp_server.url {
                    info!("  URL: {}", url);
                }
            }

            warn!("MCP servers cleared. Manual restart of MCP servers required for full reload");
            // Note: Full MCP process spawning would require CLI layer integration
        }

        info!("Configuration hot-reload completed successfully");
        Ok(())
    }
}

// Helper function to validate agent configuration
fn validate_agent_config(content: &str) -> std::result::Result<(), String> {
    // Basic validation - ensure it can be parsed
    match parse_agents_config(content) {
        Ok(agents) if agents.is_empty() => Err("No agent configurations found".to_string()),
        Ok(_) => Ok(()),
        Err(e) => Err(format!("Parse error: {e}")),
    }
}

// Simple agent configuration structure for validation
#[derive(Debug)]
struct SimpleAgentConfig {
    name: String,
    purpose: String,
    model: String,
    listen: String,
}

// Basic parser for agent configuration validation
fn parse_agents_config(content: &str) -> std::result::Result<Vec<SimpleAgentConfig>, String> {
    let mut agents = Vec::new();
    let mut current_agent: Option<SimpleAgentConfig> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        // Check for agent section header
        if trimmed.starts_with("## ") {
            // Save previous agent if exists
            if let Some(agent) = current_agent.take() {
                agents.push(agent);
            }

            let name = trimmed.strip_prefix("## ").unwrap_or("").trim().to_string();
            current_agent = Some(SimpleAgentConfig {
                name,
                purpose: String::new(),
                model: String::new(),
                listen: String::new(),
            });
        } else if let Some(ref mut agent) = current_agent {
            // Parse agent properties
            if let Some((key, value)) = trimmed.split_once(':') {
                let key = key.trim();
                let value = value.trim();

                match key {
                    "purpose" => agent.purpose = value.to_string(),
                    "model" => agent.model = value.to_string(),
                    "listen" => agent.listen = value.to_string(),
                    _ => {} // Ignore other fields for now
                }
            }
        }
    }

    // Save last agent if exists
    if let Some(agent) = current_agent {
        agents.push(agent);
    }

    Ok(agents)
}

// Helper function for file watcher to reload configuration
async fn reload_configuration_for_watcher(
    content: &str,
    agent_metadata: Arc<tokio::sync::RwLock<AgentMetadata>>,
    _llm_adapter: Arc<tokio::sync::RwLock<Option<Arc<LlmClientAdapter>>>>,
    mcp_registry: Arc<McpRegistry>,
) -> Result<()> {
    use crate::agent_config::parse_agents_config;

    // Validate configuration before applying
    if content.trim().is_empty() {
        return Err(A2aError::Configuration(
            "Configuration file is empty".to_string(),
        ));
    }

    // Parse the new configuration
    let agents = parse_agents_config(content)
        .map_err(|e| A2aError::Configuration(format!("Failed to parse configuration: {e}")))?;

    if agents.is_empty() {
        return Err(A2aError::Configuration(
            "No agent configurations found".to_string(),
        ));
    }

    // Find our agent's configuration
    let our_agent_name = agent_metadata.read().await.name.clone();

    let new_config = agents
        .iter()
        .find(|a| a.name == our_agent_name)
        .ok_or_else(|| {
            A2aError::Configuration(format!(
                "Agent '{our_agent_name}' not found in new configuration"
            ))
        })?;

    // Validate required fields
    if new_config.purpose.is_empty() {
        return Err(A2aError::Configuration(
            "Agent purpose cannot be empty".to_string(),
        ));
    }
    if new_config.model.is_empty() {
        return Err(A2aError::Configuration(
            "Agent model cannot be empty".to_string(),
        ));
    }

    // Update metadata
    {
        let mut metadata = agent_metadata.write().await;
        metadata.purpose.clone_from(&new_config.purpose);
        metadata.endpoint.clone_from(&new_config.listen);
        metadata.api_keys.clone_from(&new_config.api_keys);

        // Update environment variables
        for (key, value) in &new_config.api_keys {
            // SAFETY: We control the key and value from our config file
            unsafe {
                std::env::set_var(key, value);
            }
        }

        info!("Updated agent metadata for '{}'", metadata.name);
        info!("  Purpose: {}", metadata.purpose);
        info!("  Endpoint: {}", metadata.endpoint);
        info!("  API keys updated: {}", metadata.api_keys.len());
    }

    // Handle model changes
    if new_config.model != agent_metadata.read().await.model {
        warn!("Model change detected, but LLM adapter recreation not yet implemented");
        let mut metadata = agent_metadata.write().await;
        metadata.model.clone_from(&new_config.model);
    }

    // Handle MCP server changes
    if !new_config.mcp_servers.is_empty() {
        info!("MCP server configuration changed, clearing existing connections");

        // Clear existing MCP connections
        mcp_registry.clear_connections().await;

        // Log the MCP servers that need to be restarted
        for mcp_server in &new_config.mcp_servers {
            info!("MCP server '{}' needs restart:", mcp_server.name);
            if let Some(cmd) = &mcp_server.command {
                info!("  Command: {} {:?}", cmd, mcp_server.args);
            } else if let Some(url) = &mcp_server.url {
                info!("  URL: {}", url);
            }
        }

        warn!(
            "MCP servers cleared. Manual restart required or use agent restart for full MCP reload"
        );
    }

    Ok(())
}

// Helper function to clean up old backups
async fn cleanup_old_backups(backup_dir: &std::path::Path, keep_count: usize) {
    if let Ok(mut entries) = tokio::fs::read_dir(backup_dir).await {
        let mut backups = Vec::new();

        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Ok(metadata) = entry.metadata().await
                && metadata.is_file()
            {
                let filename = entry.file_name().to_string_lossy().to_string();
                if filename.starts_with("AGENTS.md.") && filename.ends_with(".backup") {
                    backups.push((entry.path(), metadata.modified().ok()));
                }
            }
        }

        // Sort by modification time, newest first
        backups.sort_by(|a, b| b.1.cmp(&a.1));

        // Remove old backups
        for (path, _) in backups.iter().skip(keep_count) {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
}

pub struct A2aServer {
    config: ServerConfig,
    buffer_config: BufferConfig,
    mcp_registry: Arc<McpRegistry>,
    agent_metadata: Arc<tokio::sync::RwLock<AgentMetadata>>,
    llm_adapter: Arc<tokio::sync::RwLock<Option<Arc<LlmClientAdapter>>>>,
    event_writer: Arc<tokio::sync::RwLock<Option<Arc<EventWriter>>>>,
    session_id: String,
    event_sequence: Arc<tokio::sync::RwLock<u64>>,
    file_watcher_handle: Arc<tokio::sync::RwLock<Option<tokio::task::JoinHandle<()>>>>,
    #[allow(dead_code)]
    #[allow(clippy::type_complexity)]
    mcp_reload_callback: Arc<tokio::sync::RwLock<Option<Arc<dyn Fn() + Send + Sync>>>>,
}

impl A2aServer {
    pub fn new(config: ServerConfig) -> Self {
        Self::with_buffer_config(config, BufferConfig::default())
    }

    pub fn with_buffer_config(config: ServerConfig, buffer_config: BufferConfig) -> Self {
        Self {
            config,
            buffer_config,
            mcp_registry: Arc::new(McpRegistry::new()),
            agent_metadata: Arc::new(tokio::sync::RwLock::new(AgentMetadata::default())),
            llm_adapter: Arc::new(tokio::sync::RwLock::new(None)),
            event_writer: Arc::new(tokio::sync::RwLock::new(None)),
            session_id: uuid::Uuid::new_v4().to_string(),
            event_sequence: Arc::new(tokio::sync::RwLock::new(0)),
            file_watcher_handle: Arc::new(tokio::sync::RwLock::new(None)),
            mcp_reload_callback: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    pub fn mcp_registry(&self) -> Arc<McpRegistry> {
        self.mcp_registry.clone()
    }

    pub async fn set_agent_metadata(&self, name: String, purpose: String, model: String) {
        let mut metadata = self.agent_metadata.write().await;
        metadata.name.clone_from(&name);
        metadata.purpose = purpose;
        metadata.model.clone_from(&model);
        metadata.endpoint = format!("http://{}:{}", self.config.bind_address, self.config.port);
        drop(metadata); // Release lock before creating LLM adapter

        // Create LLM adapter from model URL
        self.recreate_llm_adapter().await;
    }

    pub async fn set_api_keys(&self, api_keys: std::collections::HashMap<String, String>) {
        let mut metadata = self.agent_metadata.write().await;
        metadata.api_keys = api_keys;
        drop(metadata); // Release lock before creating LLM adapter

        // Recreate LLM adapter with new API keys
        self.recreate_llm_adapter().await;
    }

    /// Reload configuration from new content without restarting the server
    #[allow(dead_code)]
    async fn reload_configuration_from_content(&self, content: &str) -> Result<()> {
        use crate::agent_config::parse_agents_config;

        // Parse the new configuration
        let configs = parse_agents_config(content)
            .map_err(|e| A2aError::Configuration(format!("Failed to parse config: {e}")))?;

        // Get current agent name to find matching config
        let current_name = {
            let metadata = self.agent_metadata.read().await;
            metadata.name.clone()
        };

        // Find the configuration for this agent
        let new_config = configs
            .iter()
            .find(|c| c.name == current_name)
            .ok_or_else(|| {
                A2aError::Configuration(format!(
                    "Agent '{current_name}' not found in updated configuration"
                ))
            })?;

        // Update agent metadata (name, purpose, model)
        info!(
            "Updating agent metadata: name={}, purpose={}, model={}",
            new_config.name, new_config.purpose, new_config.model
        );

        // Check if model changed
        let model_changed = {
            let metadata = self.agent_metadata.read().await;
            metadata.model != new_config.model
        };

        // Update metadata
        self.set_agent_metadata(
            new_config.name.clone(),
            new_config.purpose.clone(),
            new_config.model.clone(),
        )
        .await;

        // Update API keys if present
        if !new_config.api_keys.is_empty() {
            info!("Updating API keys");
            self.set_api_keys(new_config.api_keys.clone()).await;
        }

        // If model changed, LLM adapter was already recreated by set_agent_metadata
        if model_changed {
            info!("Model changed, LLM adapter recreated");
        }

        // TODO: Handle MCP server updates in Phase 3
        // This will require access to McpProcessManager and careful state management
        if !new_config.mcp_servers.is_empty() {
            warn!(
                "MCP server configuration changes detected. Full hot-reload for MCP servers will be implemented in Phase 3."
            );
            warn!("For now, MCP server changes require agent restart to take effect.");
        }

        // TODO: Handle listen address changes
        // Changing the listen address would require rebinding the server socket
        let current_listen = format!("{}:{}", self.config.bind_address, self.config.port);
        if new_config.listen != current_listen && new_config.listen != "0.0.0.0:0" {
            warn!(
                "Listen address changes require agent restart to take effect (current: {}, new: {})",
                current_listen, new_config.listen
            );
        }

        info!("Configuration reloaded successfully");
        Ok(())
    }

    /// Initialize the event writer for debugging
    pub async fn initialize_event_writer(&self) -> Result<()> {
        use arkavo_events::writer::EventWriterBuilder;
        use std::time::Duration;

        let config = EventWriterConfig {
            buffer_size: 10_000,
            flush_interval: Duration::from_millis(100),
            batch_size: 200,
        };

        let writer = EventWriterBuilder::new()
            .with_config(config)
            .add_handler(move |events| {
                // Log events for now - will be sent to debug handler later
                for event in events {
                    tracing::debug!(
                        event_type = %event.event_type(),
                        session_id = %event.session_id,
                        "Event captured"
                    );
                }
            })
            .build();

        *self.event_writer.write().await = Some(Arc::new(writer));

        // Emit session started event
        self.emit_session_started().await?;

        Ok(())
    }

    /// Get the session ID for this server instance
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Get next event sequence number
    async fn next_sequence(&self) -> u64 {
        let mut seq = self.event_sequence.write().await;
        let current = *seq;
        *seq += 1;
        current
    }

    /// Emit a session started event
    async fn emit_session_started(&self) -> Result<()> {
        if let Some(writer) = self.event_writer.read().await.as_ref() {
            let metadata = self.agent_metadata.read().await;
            let capabilities = vec![
                "a2a-protocol".to_string(),
                "mcp-integration".to_string(),
                "chat-streaming".to_string(),
            ];

            let sequence = self.next_sequence().await;
            let event = Event::new(
                self.session_id.clone(),
                sequence,
                metadata.name.clone(),
                EventPayload::SessionStarted {
                    capabilities: Some(capabilities),
                    metadata: Some(
                        [
                            (
                                "model".to_string(),
                                serde_json::Value::String(metadata.model.clone()),
                            ),
                            (
                                "purpose".to_string(),
                                serde_json::Value::String(metadata.purpose.clone()),
                            ),
                            (
                                "endpoint".to_string(),
                                serde_json::Value::String(metadata.endpoint.clone()),
                            ),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                },
            );

            writer.write(event).await.map_err(|e| {
                A2aError::Internal(format!("Failed to write session started event: {e}"))
            })?;
        }

        Ok(())
    }

    async fn recreate_llm_adapter(&self) {
        let metadata = self.agent_metadata.read().await;
        let model = metadata.model.clone();
        let api_keys = metadata.api_keys.clone();
        drop(metadata); // Release lock before creating adapter

        if let Ok(adapter) = self.create_llm_adapter(&model, &api_keys) {
            *self.llm_adapter.write().await = Some(adapter);
            info!("Created LLM adapter with model: {}", model);
        } else {
            error!(model = model, "Failed to create LLM adapter for model");
        }
    }

    fn create_llm_adapter(
        &self,
        model_url: &str,
        api_keys: &std::collections::HashMap<String, String>,
    ) -> Result<Arc<LlmClientAdapter>> {
        // Parse the model URL to extract provider and configuration
        // Format: provider://host:port/model
        if let Some((provider, rest)) = model_url.split_once("://") {
            match provider {
                "ollama" => {
                    // Set environment variables for Ollama client
                    if let Some((host_port, model_name)) = rest.rsplit_once('/') {
                        unsafe {
                            std::env::set_var("LLM_PROVIDER", "ollama");
                            std::env::set_var("OLLAMA_BASE_URL", format!("http://{host_port}"));
                            std::env::set_var("OLLAMA_MODEL", model_name);
                        }

                        // Create LLM client from environment
                        let client = LlmClient::from_env().map_err(|e| {
                            A2aError::InvalidRequest(format!("Failed to create LLM client: {e}"))
                        })?;
                        Ok(Arc::new(LlmClientAdapter::new(client)))
                    } else {
                        Err(A2aError::InvalidRequest(format!(
                            "Invalid Ollama URL format: {model_url}"
                        )))
                    }
                }
                "kimi" => {
                    // KIMI format: kimi://model-name (e.g., kimi://moonshot-v1-128k)
                    unsafe {
                        std::env::set_var("LLM_PROVIDER", "kimi");
                        // Extract model name from rest (e.g., moonshot-v1-128k)
                        if !rest.is_empty() {
                            // Model name is already in the correct format
                            std::env::set_var("KIMI_MODEL", rest);
                        }

                        // Set API key from agent config if available
                        if let Some(api_key) = api_keys.get("MOONSHOT_API_KEY") {
                            std::env::set_var("MOONSHOT_API_KEY", api_key);
                        }
                    }
                    // Create LLM client from environment
                    let client = LlmClient::from_env().map_err(|e| {
                        A2aError::InvalidRequest(format!("Failed to create KIMI client: {e}"))
                    })?;
                    Ok(Arc::new(LlmClientAdapter::new(client)))
                }
                _ => Err(A2aError::InvalidRequest(format!(
                    "Unsupported LLM provider: {provider}"
                ))),
            }
        } else {
            Err(A2aError::InvalidRequest(format!(
                "Invalid model URL format: {model_url}"
            )))
        }
    }

    /// Start file watcher for AGENTS.md hot-reload
    pub async fn start_file_watcher(&self) -> Result<()> {
        use notify::{Event, EventKind, RecursiveMode, Watcher};
        use std::sync::mpsc::channel;
        use std::time::Duration;

        // Stop any existing watcher
        self.stop_file_watcher().await;

        let config_path = std::path::Path::new("AGENTS.md");
        if !config_path.exists() {
            info!("AGENTS.md not found, skipping file watcher setup");
            return Ok(());
        }

        let (tx, rx) = channel();
        let mut watcher = notify::recommended_watcher(tx)
            .map_err(|e| A2aError::Internal(format!("Failed to create file watcher: {e}")))?;

        // Watch the AGENTS.md file
        watcher
            .watch(config_path, RecursiveMode::NonRecursive)
            .map_err(|e| A2aError::Internal(format!("Failed to watch AGENTS.md: {e}")))?;

        info!("File watcher started for AGENTS.md");

        // Clone the necessary components for hot-reload
        let agent_metadata = self.agent_metadata.clone();
        let llm_adapter = self.llm_adapter.clone();
        let mcp_registry = self.mcp_registry.clone();

        // Spawn watcher task with blocking recv
        let handle = tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Handle::current();
            let mut last_reload = std::time::Instant::now();
            let _watcher = watcher; // Keep watcher alive

            loop {
                match rx.recv_timeout(Duration::from_secs(1)) {
                    Ok(Ok(Event {
                        kind: EventKind::Modify(_),
                        ..
                    })) => {
                        // Debounce: only reload if at least 1 second has passed
                        if last_reload.elapsed() > Duration::from_secs(1) {
                            info!("AGENTS.md modified, triggering hot-reload");

                            // Read the new content and reload in async context
                            let agent_metadata_clone = agent_metadata.clone();
                            let llm_adapter_clone = llm_adapter.clone();
                            let mcp_registry_clone = mcp_registry.clone();
                            #[allow(clippy::disallowed_methods)]
                            rt.block_on(async move {
                                match tokio::fs::read_to_string("AGENTS.md").await {
                                    Ok(content) => {
                                        // Perform hot-reload directly here
                                        match reload_configuration_for_watcher(
                                            &content,
                                            agent_metadata_clone,
                                            llm_adapter_clone,
                                            mcp_registry_clone
                                        ).await {
                                            Ok(_) => {
                                                info!("Configuration hot-reload completed successfully");
                                            }
                                            Err(e) => {
                                                error!("Configuration hot-reload failed: {}", e);
                                                error!("Agent will continue with existing configuration");
                                                // Could trigger recovery mechanism here
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        error!("Failed to read AGENTS.md for hot-reload: {}", e);
                                    }
                                }
                            });
                            last_reload = std::time::Instant::now();
                        }
                    }
                    Ok(Err(e)) => {
                        warn!("File watcher error: {}", e);
                    }
                    Err(_) => {
                        // Timeout, continue loop
                    }
                    _ => {}
                }
            }
        });

        // Store the handle
        *self.file_watcher_handle.write().await = Some(handle);

        Ok(())
    }

    /// Stop the file watcher
    pub async fn stop_file_watcher(&self) {
        if let Some(handle) = self.file_watcher_handle.write().await.take() {
            handle.abort();
            info!("File watcher stopped");
        }
    }

    pub async fn start(&self) -> Result<ServerHandle> {
        let addr: SocketAddr = format!("{}:{}", self.config.bind_address, self.config.port)
            .parse()
            .map_err(|e| A2aError::InvalidEndpoint(format!("Invalid bind address: {e}")))?;

        info!("Starting A2A server on {}", addr);

        let server = ServerBuilder::default()
            .max_connections(self.config.max_connections as u32)
            .build(addr)
            .await
            .map_err(|e| A2aError::Transport(format!("Failed to build server: {e}")))?;

        let rate_limiter = Arc::new(RateLimiter::new(self.config.rate_limit.clone()));
        let metrics = Arc::new(MetricsCollector::new(self.config.metrics_enabled));
        let llm_adapter = self.llm_adapter.read().await.clone();
        let chat_sessions = Arc::new(crate::chat_session::ChatSessionManager::with_config(
            llm_adapter.clone(),
            3600, // 1 hour TTL
            self.buffer_config.clone(),
        ));

        // Create task store and executor
        let task_store: Arc<dyn TaskStore> =
            match &self.config.task_store_path {
                Some(path) => {
                    let task_store_path = std::path::Path::new(path);
                    Arc::new(SqliteTaskStore::new(task_store_path).await.map_err(|e| {
                        A2aError::Internal(format!("Failed to create task store: {e}"))
                    })?)
                }
                None => {
                    // Use in-memory database
                    Arc::new(SqliteTaskStore::new_in_memory().await.map_err(|e| {
                        A2aError::Internal(format!("Failed to create in-memory task store: {e}"))
                    })?)
                }
            };
        let task_executor = Arc::new(TaskExecutor::with_metrics(
            task_store.clone(),
            TaskExecutorConfig::default(),
            metrics.clone(),
        ));

        // Start the task executor
        task_executor
            .start()
            .map_err(|e| A2aError::Internal(format!("Failed to start task executor: {e}")))?;

        let rpc_impl = A2aRpcImpl {
            rate_limiter,
            metrics,
            mcp_registry: self.mcp_registry.clone(),
            agent_metadata: self.agent_metadata.clone(),
            llm_adapter,
            chat_sessions,
            task_store,
            task_executor,
            event_writer: self.event_writer.read().await.clone(),
            session_id: self.session_id.clone(),
            event_sequence: self.event_sequence.clone(),
            auth_backend: Arc::new(NoOpAuthBackend),
        };

        // Start file watcher for hot-reload
        if let Err(e) = self.start_file_watcher().await {
            warn!("Failed to start file watcher: {}", e);
            // Continue without file watcher - not a critical failure
        }

        let handle = server.start(rpc_impl.into_rpc());

        info!("A2A server started successfully on {}", addr);
        info!("OpenRPC schema available via JSON-RPC method: rpc.discover");

        Ok(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_server_creation() {
        let config = ServerConfig::default();
        let _server = A2aServer::new(config);
        // Server creation should succeed
    }

    #[tokio::test]
    async fn test_task_request_handler() {
        let mut config = crate::rate_limit::RateLimitConfig::default();
        config.max_requests_per_second = 100;
        let rate_limiter = Arc::new(crate::rate_limit::RateLimiter::new(config));
        let metrics = Arc::new(MetricsCollector::new(false));
        let task_store: Arc<dyn TaskStore> =
            Arc::new(SqliteTaskStore::new_in_memory().await.unwrap());
        let task_executor = Arc::new(TaskExecutor::with_metrics(
            task_store.clone(),
            TaskExecutorConfig::default(),
            metrics.clone(),
        ));
        task_executor.start().unwrap();
        let impl_instance = A2aRpcImpl {
            rate_limiter,
            metrics,
            mcp_registry: Arc::new(McpRegistry::new()),
            agent_metadata: Arc::new(tokio::sync::RwLock::new(AgentMetadata::default())),
            llm_adapter: None,
            chat_sessions: Arc::new(crate::chat_session::ChatSessionManager::new(None)),
            task_store,
            task_executor,
            event_writer: None,
            session_id: uuid::Uuid::new_v4().to_string(),
            event_sequence: Arc::new(tokio::sync::RwLock::new(0)),
            auth_backend: Arc::new(NoOpAuthBackend),
        };
        let result = impl_instance
            .task_request(
                "test-agent".to_string(),
                "data_access".to_string(),
                Some(serde_json::json!({"key": "value"})),
            )
            .await;

        #[cfg(feature = "stub_handlers")]
        {
            let result = result.unwrap();
            assert!(result.task_id.to_string().len() > 0);
            assert!(matches!(result.status, TaskStatus::Submitted));
        }

        #[cfg(not(feature = "stub_handlers"))]
        {
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.code(), -32601);
            assert!(err.message().contains("not yet implemented"));
        }
    }

    #[tokio::test]
    async fn test_task_declare_handler() {
        let mut config = crate::rate_limit::RateLimitConfig::default();
        config.max_requests_per_second = 100;
        let rate_limiter = Arc::new(crate::rate_limit::RateLimiter::new(config));
        let metrics = Arc::new(MetricsCollector::new(false));
        let task_store: Arc<dyn TaskStore> =
            Arc::new(SqliteTaskStore::new_in_memory().await.unwrap());
        let task_executor = Arc::new(TaskExecutor::with_metrics(
            task_store.clone(),
            TaskExecutorConfig::default(),
            metrics.clone(),
        ));
        task_executor.start().unwrap();
        let impl_instance = A2aRpcImpl {
            rate_limiter,
            metrics,
            mcp_registry: Arc::new(McpRegistry::new()),
            agent_metadata: Arc::new(tokio::sync::RwLock::new(AgentMetadata::default())),
            llm_adapter: None,
            chat_sessions: Arc::new(crate::chat_session::ChatSessionManager::new(None)),
            task_store,
            task_executor,
            event_writer: None,
            session_id: uuid::Uuid::new_v4().to_string(),
            event_sequence: Arc::new(tokio::sync::RwLock::new(0)),
            auth_backend: Arc::new(NoOpAuthBackend),
        };
        let result = impl_instance
            .task_declare(
                "test-agent".to_string(),
                vec![TaskCapability {
                    task_type: "compute".to_string(),
                    constraints: None,
                    metadata: None,
                }],
            )
            .await;

        #[cfg(feature = "stub_handlers")]
        {
            let result = result.unwrap();
            assert!(result.acknowledged);
            assert!(result.timestamp <= chrono::Utc::now());
        }

        #[cfg(not(feature = "stub_handlers"))]
        {
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.code(), -32601);
            assert!(err.message().contains("not yet implemented"));
        }
    }

    #[tokio::test]
    async fn test_rpc_discover() {
        let mut config = crate::rate_limit::RateLimitConfig::default();
        config.max_requests_per_second = 100;
        let rate_limiter = Arc::new(crate::rate_limit::RateLimiter::new(config));
        let metrics = Arc::new(MetricsCollector::new(false));
        let task_store: Arc<dyn TaskStore> =
            Arc::new(SqliteTaskStore::new_in_memory().await.unwrap());
        let task_executor = Arc::new(TaskExecutor::with_metrics(
            task_store.clone(),
            TaskExecutorConfig::default(),
            metrics.clone(),
        ));
        task_executor.start().unwrap();
        let impl_instance = A2aRpcImpl {
            rate_limiter,
            metrics,
            mcp_registry: Arc::new(McpRegistry::new()),
            agent_metadata: Arc::new(tokio::sync::RwLock::new(AgentMetadata::default())),
            llm_adapter: None,
            chat_sessions: Arc::new(crate::chat_session::ChatSessionManager::new(None)),
            task_store,
            task_executor,
            event_writer: None,
            session_id: uuid::Uuid::new_v4().to_string(),
            event_sequence: Arc::new(tokio::sync::RwLock::new(0)),
            auth_backend: Arc::new(NoOpAuthBackend),
        };
        let result = impl_instance.rpc_discover().await.unwrap();

        assert_eq!(result.get("openrpc").unwrap(), "1.2.6");
        assert!(result.get("info").is_some());
        assert!(result.get("methods").is_some());

        let methods = result.get("methods").unwrap().as_array().unwrap();
        assert!(methods.len() >= 3);

        let method_names: Vec<&str> = methods
            .iter()
            .filter_map(|m| m.get("name").and_then(|n| n.as_str()))
            .collect();

        assert!(method_names.contains(&"task_request"));
        assert!(method_names.contains(&"task_declare"));
        assert!(method_names.contains(&"agent_discover"));
    }

    #[tokio::test]
    async fn test_agent_discover_handler() {
        let mut config = crate::rate_limit::RateLimitConfig::default();
        config.max_requests_per_second = 100;
        let rate_limiter = Arc::new(crate::rate_limit::RateLimiter::new(config));
        let metrics = Arc::new(MetricsCollector::new(false));
        let task_store: Arc<dyn TaskStore> =
            Arc::new(SqliteTaskStore::new_in_memory().await.unwrap());
        let task_executor = Arc::new(TaskExecutor::with_metrics(
            task_store.clone(),
            TaskExecutorConfig::default(),
            metrics.clone(),
        ));
        task_executor.start().unwrap();
        let impl_instance = A2aRpcImpl {
            rate_limiter,
            metrics,
            mcp_registry: Arc::new(McpRegistry::new()),
            agent_metadata: Arc::new(tokio::sync::RwLock::new(AgentMetadata::default())),
            llm_adapter: None,
            chat_sessions: Arc::new(crate::chat_session::ChatSessionManager::new(None)),
            task_store,
            task_executor,
            event_writer: None,
            session_id: uuid::Uuid::new_v4().to_string(),
            event_sequence: Arc::new(tokio::sync::RwLock::new(0)),
            auth_backend: Arc::new(NoOpAuthBackend),
        };
        let result = impl_instance
            .agent_discover(Some(AgentDiscoverFilter {
                task_types: Some(vec!["test".to_string()]),
                tags: None,
            }))
            .await;

        // agent_discover now returns actual data regardless of stub_handlers feature
        let result = result.unwrap();
        assert_eq!(result.len(), 1);
        let agent = &result[0];
        assert!(!agent.agent_id.to_string().is_empty());
        assert!(agent.metadata.is_some());
    }

    #[tokio::test]
    async fn test_rate_limiting() {
        let mut config = crate::rate_limit::RateLimitConfig::default();
        config.max_requests_per_second = 1;
        config.burst_size = 1;
        let rate_limiter = Arc::new(crate::rate_limit::RateLimiter::new(config));
        let metrics = Arc::new(MetricsCollector::new(false));
        let task_store: Arc<dyn TaskStore> =
            Arc::new(SqliteTaskStore::new_in_memory().await.unwrap());
        let task_executor = Arc::new(TaskExecutor::with_metrics(
            task_store.clone(),
            TaskExecutorConfig::default(),
            metrics.clone(),
        ));
        task_executor.start().unwrap();
        let impl_instance = A2aRpcImpl {
            rate_limiter,
            metrics,
            mcp_registry: Arc::new(McpRegistry::new()),
            agent_metadata: Arc::new(tokio::sync::RwLock::new(AgentMetadata::default())),
            llm_adapter: None,
            chat_sessions: Arc::new(crate::chat_session::ChatSessionManager::new(None)),
            task_store,
            task_executor,
            event_writer: None,
            session_id: uuid::Uuid::new_v4().to_string(),
            event_sequence: Arc::new(tokio::sync::RwLock::new(0)),
            auth_backend: Arc::new(NoOpAuthBackend),
        };

        // First request should succeed
        let result1 = impl_instance.agent_discover(None).await;
        assert!(result1.is_ok());

        // Second request should be rate limited
        let result2 = impl_instance.agent_discover(None).await;

        assert!(result2.is_err());
        let err = result2.unwrap_err();
        assert_eq!(err.code(), -32001);
        assert!(err.message().contains("Rate limit"));
    }
}
