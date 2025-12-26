use crate::mcp_registry::McpRegistry;
use crate::metrics::{MetricsCollector, RpcTimer};
use crate::rate_limit::RateLimiter;
use crate::task_executor::TaskExecutor;
use crate::task_store::TaskStore;
use crate::types::{
    AgentBroadcast, AgentQueryRequest, AgentQueryResponse, BroadcastType, Message,
    MessageSendRequest, MessageSendResponse, TaskStatus,
};
use arkavo_events::{Event, EventPayload, EventWriter};
use arkavo_hrm::{Conductor, store::InMemoryTaskStore};
use jsonrpsee::types::ErrorObjectOwned;
use std::sync::Arc;
use tracing::{info, warn};

use super::super::LearningBus;
use super::super::config_helpers::AgentMetadata;
use super::super::execute_with_conductor_and_learning;

#[allow(clippy::too_many_arguments)]
pub async fn handle_message_send(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    task_executor: &Arc<TaskExecutor>,
    task_store: &Arc<dyn TaskStore>,
    mcp_registry: &Arc<McpRegistry>,
    conductor: &Arc<Conductor<InMemoryTaskStore>>,
    router: Option<&Arc<arkavo_router::Router>>,
    learning_bus: Option<&Arc<LearningBus>>,
    request: MessageSendRequest,
) -> Result<MessageSendResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("message/send".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    let task_content: String = request
        .message
        .parts
        .iter()
        .filter_map(|p| match p {
            crate::types::MessagePart::Text { content } => Some(content.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    match task_executor.submit_task(request.message).await {
        Ok(task_id) => {
            if let Some(router) = router {
                let router = router.clone();
                let conductor = conductor.clone();
                let mcp_registry = mcp_registry.clone();
                let task_executor = task_executor.clone();
                let task_store = task_store.clone();
                let task_id_clone = task_id;
                let learning_bus = learning_bus.cloned();

                tokio::spawn(async move {
                    if let Err(e) = task_executor
                        .update_task_status(&task_id_clone, TaskStatus::Working)
                        .await
                    {
                        warn!("Failed to update task {} to Working: {}", task_id_clone, e);
                        return;
                    }

                    info!("Executing task {} via HRM Conductor", task_id_clone);

                    match execute_with_conductor_and_learning(
                        &conductor,
                        &router,
                        &mcp_registry,
                        task_content,
                        Some(task_id_clone),
                        Some(&task_executor),
                        learning_bus.as_ref(),
                    )
                    .await
                    {
                        Ok(result_content) => {
                            let result_message = Message {
                                parts: vec![crate::types::MessagePart::Text {
                                    content: result_content,
                                }],
                                metadata: None,
                            };

                            if let Err(e) = task_executor
                                .complete_task(&task_id_clone, result_message)
                                .await
                            {
                                warn!("Failed to complete task {}: {}", task_id_clone, e);
                            } else {
                                info!("Task {} completed successfully via HRM", task_id_clone);
                            }
                        }
                        Err(error_msg) => {
                            let error = crate::types::TaskError {
                                code: "HRM_EXECUTION_ERROR".to_string(),
                                message: error_msg.clone(),
                                details: None,
                            };

                            if let Ok(Some(mut task)) = task_store.get_task(&task_id_clone).await {
                                task.error = Some(error);
                                let _ = task_store.create_task(task).await;
                            }

                            if let Err(e) = task_executor
                                .update_task_status(&task_id_clone, TaskStatus::Failed)
                                .await
                            {
                                warn!("Failed to mark task {} as failed: {}", task_id_clone, e);
                            } else {
                                warn!("Task {} failed: {}", task_id_clone, error_msg);
                            }
                        }
                    }
                });
            }

            let response = MessageSendResponse {
                task_id: task_id.to_string(),
                status: TaskStatus::Submitted,
                response: None,
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

pub async fn handle_agent_query(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    mcp_registry: &Arc<McpRegistry>,
    agent_metadata: &Arc<tokio::sync::RwLock<AgentMetadata>>,
    request: AgentQueryRequest,
) -> Result<AgentQueryResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("agent_query".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    let metadata = agent_metadata.read().await;
    let agent_id = metadata.name.clone();
    drop(metadata);

    if let Some(ref target_id) = request.to_agent_id {
        let agents = mcp_registry.list_agents().await;
        if !agents.iter().any(|a| &a.identity.id == target_id) {
            timer.error();
            return Err(jsonrpsee::types::error::ErrorObject::owned(
                -32001,
                format!("Target agent not found: {target_id}"),
                None::<()>,
            ));
        }
    }

    let mut response_text = format!("Processing query: {}", request.query);
    let mut confidence = 0.5;
    let mut evidence = None;

    if let Some(ref domain) = request.domain {
        match domain.as_str() {
            "code" => {
                response_text = format!("Code analysis for: {}", request.query);
                confidence = 0.7;
                evidence = Some(serde_json::json!({"domain": "code", "analyzed": true}));
            }
            "data" => {
                response_text = format!("Data query result for: {}", request.query);
                confidence = 0.8;
                evidence = Some(serde_json::json!({"domain": "data", "records_examined": 100}));
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

#[allow(clippy::too_many_arguments)]
pub async fn handle_agent_broadcast(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    mcp_registry: &Arc<McpRegistry>,
    agent_metadata: &Arc<tokio::sync::RwLock<AgentMetadata>>,
    event_writer: Option<&Arc<EventWriter>>,
    session_id: &str,
    event_sequence: &Arc<tokio::sync::RwLock<u64>>,
    broadcast: AgentBroadcast,
) -> Result<(), ErrorObjectOwned> {
    let timer = RpcTimer::new("agent_broadcast".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    if matches!(broadcast.broadcast_type, BroadcastType::Capability)
        && let Some(ref capabilities) = broadcast.capabilities
    {
        for capability in capabilities {
            info!(
                "Agent {} announced capability: {} - {:?}",
                broadcast.agent_id, capability.name, capability.description
            );
        }
    }

    match broadcast.broadcast_type {
        BroadcastType::Status => {
            info!(
                "Agent {} status broadcast: {:?}",
                broadcast.agent_id, broadcast.status
            );

            if let Some(ref status) = broadcast.status {
                mcp_registry
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
            mcp_registry.unregister_agent(&broadcast.agent_id).await;
        }
        BroadcastType::Custom(ref custom_type) => {
            info!(
                "Agent {} custom broadcast type: {}",
                broadcast.agent_id, custom_type
            );
        }
    }

    if let Some(event_writer) = event_writer {
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

        let agent_meta = agent_metadata.read().await;
        let event = Event::new(
            session_id.to_string(),
            *event_sequence.read().await,
            agent_meta.name.clone(),
            event_payload,
        );
        drop(agent_meta);

        *event_sequence.write().await += 1;

        if let Err(e) = event_writer.write(event).await {
            warn!("Failed to write broadcast event: {}", e);
        }
    }

    timer.success();
    Ok(())
}
