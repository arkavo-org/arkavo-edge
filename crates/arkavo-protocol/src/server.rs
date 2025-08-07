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
    AgentBroadcast, AgentDiscoverFilter, AgentQueryRequest, AgentQueryResponse, ChatOpenRequest,
    ChatRequest, ChatSession, DiscoverFeaturesDisclose, DiscoverFeaturesQuery, DiscoveredAgent,
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
use tracing::{error, info};

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

        let agent = DiscoveredAgent {
            agent_id: uuid::Uuid::new_v4(), // Generate a unique ID for the agent
            endpoint,
            tasks: Some(vec![]), // TODO: Populate with actual task types
            metadata: Some(metadata_json),
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
        if let Some(query) = query {
            if let Some(queries) = query.queries {
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
                    if let Ok(msg) = SubscriptionMessage::from_json(&delta) {
                        if sink.send(msg).await.is_err() {
                            break; // Client disconnected
                        }
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
                                    {
                                        if sink.send(msg).await.is_err() {
                                            break; // Client disconnected
                                        }
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

        // TODO: Implement agent query logic
        // For now, return a placeholder response
        let response = AgentQueryResponse {
            from_agent_id: self.agent_metadata.read().await.name.clone(),
            response: format!("Response to query: {}", request.query),
            confidence: 0.8,
            domain: request.domain,
            evidence: None,
        };

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

        // TODO: Implement broadcast logic
        // For now, just log the broadcast
        info!(
            agent_id = broadcast.agent_id,
            broadcast_type = ?broadcast.broadcast_type,
            capabilities = ?broadcast.capabilities,
            "Agent broadcast received"
        );

        timer.success();
        Ok(())
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
        let metrics = Arc::new(MetricsCollector::new(true)); // TODO: Make configurable
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
