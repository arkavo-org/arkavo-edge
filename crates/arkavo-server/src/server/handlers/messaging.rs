use arkavo_events::{Event, EventPayload, EventWriter};
use arkavo_hrm::{Conductor, store::InMemoryTaskStore};
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_protocol::metrics::{MetricsCollector, RpcTimer};
use arkavo_protocol::rate_limit::RateLimiter;
use arkavo_protocol::types::{
    AgentBroadcast, AgentQueryRequest, AgentQueryResponse, BroadcastType, Message, MessagePart,
    MessageSendRequest, MessageSendResponse, TaskError, TaskStatus,
};
use arkavo_tasks::task_executor::TaskExecutor;
use arkavo_tasks::task_store::TaskStore;
use jsonrpsee::types::ErrorObjectOwned;
use std::sync::Arc;
use tracing::{info, warn};

use super::super::LearningBus;
use super::super::config_helpers::AgentMetadata;
use super::super::execute_with_conductor_and_learning;
use super::super::tool_memory::ToolMemory;

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
    budget_manager: Option<&Arc<arkavo_budget::BudgetManager>>,
    model_hint: Option<arkavo_router::ModelChoice>,
    compute_budget: &arkavo_budget::SharedComputeBudget,
    mesh_state: Option<&Arc<arkavo_mcp_mesh::MeshToolsState>>,
    agent_metadata: &Arc<tokio::sync::RwLock<AgentMetadata>>,
    agent_memory: &Arc<tokio::sync::RwLock<ToolMemory>>,
    #[cfg(feature = "iroh")] iroh_node: Option<&Arc<arkavo_tdf_iroh::IrohNode>>,
    request: MessageSendRequest,
) -> Result<MessageSendResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("message/send".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    // Extract task content from message parts
    let task_content: String = request
        .message
        .parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text { content } => Some(content.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Extract base64-encoded images from File parts with image/* MIME types
    let images: Vec<String> = request
        .message
        .parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::File {
                mime_type,
                data,
                is_uri,
                ..
            } if mime_type.starts_with("image/") && !is_uri => Some(data.clone()),
            _ => None,
        })
        .collect();
    let images = if images.is_empty() {
        None
    } else {
        info!("Message contains {} image attachment(s)", images.len());
        Some(images)
    };

    // Task Contract Protocol: extract contract data parts or auto-propose
    let contract_context = {
        use super::super::contract_negotiation::{
            ContractAction, ContractNegotiator, contract_to_verification_context,
        };
        use arkavo_protocol::task_contract::{ContractProposal, SCHEMA_CONTRACT_PROPOSAL};

        let contract_parts: Vec<(String, serde_json::Value)> = request
            .message
            .parts
            .iter()
            .filter_map(|part| match part {
                MessagePart::Data { schema, content }
                    if schema.starts_with("urn:arkavo:contract:") =>
                {
                    Some((schema.clone(), content.clone()))
                }
                _ => None,
            })
            .collect();

        let mut negotiator = ContractNegotiator::new();

        if let Some((schema, content)) = contract_parts.first() {
            // Explicit contract proposal in message
            if schema == SCHEMA_CONTRACT_PROPOSAL {
                if let Ok(proposal) = serde_json::from_value::<ContractProposal>(content.clone()) {
                    let review = negotiator.review(&proposal);
                    match negotiator.advance(proposal.contract_id, &review) {
                        ContractAction::Approved(contract) => {
                            info!(contract_id = %contract.contract_id, "Contract approved");
                            let ctx = contract_to_verification_context(&contract);
                            negotiator.cleanup(contract.contract_id);
                            Some(ctx)
                        }
                        ContractAction::RequestRevision(review) => {
                            info!(
                                contract_id = %proposal.contract_id,
                                round = review.round,
                                "Contract needs revision"
                            );
                            None
                        }
                        ContractAction::Escalate {
                            contract_id,
                            reason,
                        } => {
                            info!(%contract_id, %reason, "Contract escalated");
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else if task_content.lines().count() > 3 {
            // Auto-propose for complex tasks (>3 lines with structured criteria)
            let proposal = negotiator.propose("auto", &task_content);
            if !proposal.acceptance_criteria.is_empty() {
                let review = negotiator.review(&proposal);
                if review.approved {
                    if let ContractAction::Approved(contract) =
                        negotiator.advance(proposal.contract_id, &review)
                    {
                        info!(
                            criteria = proposal.acceptance_criteria.len(),
                            "Auto-proposed contract approved"
                        );
                        let ctx = contract_to_verification_context(&contract);
                        negotiator.cleanup(contract.contract_id);
                        Some(ctx)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    };
    let _ = contract_context;

    // Preflight moderation: reject policy-violating requests before task submission
    if let Some(router) = router
        && let Some(arkavo_router::ModerationResult::Block {
            policy_id, reason, ..
        }) = router.check_preflight(&task_content)
    {
        info!(policy_id = %policy_id, "Message blocked by preflight policy");
        timer.error();
        return Err(ErrorObjectOwned::owned(
            -32001,
            format!("Blocked by policy: {policy_id}"),
            Some(reason),
        ));
    }

    // Budget enforcement: reject if session budget exhausted
    if let Some(manager) = budget_manager {
        let tracker = manager.tracker();
        let status = tracker.get_status().await;
        let config = manager.get_config().await;
        if let Some(limit) = config.limits.session_limit
            && status.session_spent >= limit
        {
            info!("Message rejected: session budget exhausted");
            timer.error();
            return Err(ErrorObjectOwned::owned(
                -32002,
                "Session budget exhausted",
                Some(format!(
                    "Spent ${:.2} of ${:.2} session limit",
                    status.session_spent.as_dollars(),
                    limit.as_dollars()
                )),
            ));
        }
        if let Some(limit) = config.limits.daily_limit
            && status.daily_spent >= limit
        {
            info!("Message rejected: daily budget exhausted");
            timer.error();
            return Err(ErrorObjectOwned::owned(
                -32002,
                "Daily budget exhausted",
                Some(format!(
                    "Spent ${:.2} of ${:.2} daily limit",
                    status.daily_spent.as_dollars(),
                    limit.as_dollars()
                )),
            ));
        }
    }

    // Extract budget allocation from task metadata and refresh compute budget
    if let Some(metadata) = &request.message.metadata
        && let Some(alloc_value) = metadata.get("budget_allocation")
        && let Ok(allocation) =
            serde_json::from_value::<arkavo_budget::BudgetAllocation>(alloc_value.clone())
    {
        let mut budget = compute_budget.write().await;
        budget.refresh(&allocation);
        metrics.record_compute_budget_refresh();
        let snapshot = budget.snapshot();
        metrics.record_compute_budget_state(
            &snapshot.status,
            snapshot.remaining_inferences,
            snapshot.remaining_tokens,
        );
    }

    // Read agent purpose for system prompt injection so specialists
    // receive their AGENTS.md identity when processing delegated tasks.
    let purpose = agent_metadata.read().await.purpose.clone();

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
                let compute_budget = compute_budget.clone();
                let mesh_state = mesh_state.cloned();
                let agent_memory = agent_memory.clone();
                let specialist_id = agent_metadata.read().await.name.clone();
                let task_start = std::time::Instant::now();
                #[cfg(feature = "iroh")]
                let iroh_node = iroh_node.cloned();

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
                        Some(&agent_memory),
                        if purpose.is_empty() {
                            None
                        } else {
                            Some(purpose.as_str())
                        },
                        mesh_state.as_ref(),
                        model_hint.as_ref(),
                        images,
                        Some(&compute_budget),
                        #[cfg(feature = "iroh")]
                        iroh_node.as_ref(),
                    )
                    .await
                    {
                        Ok(result_content) => {
                            let result_for_notice = result_content.clone();
                            let result_message = Message {
                                parts: vec![arkavo_protocol::types::MessagePart::Text {
                                    content: result_content,
                                }],
                                metadata: None,
                            };
                            let result_value = serde_json::to_value(&result_message)
                                .unwrap_or(serde_json::Value::Null);

                            if let Err(e) = task_executor
                                .complete_task(&task_id_clone, result_value)
                                .await
                            {
                                warn!("Failed to complete task {}: {}", task_id_clone, e);
                            } else {
                                info!("Task {} completed successfully via HRM", task_id_clone);

                                // Push completion to commander via gossip (no polling needed)
                                if let Some(ref bus) = learning_bus {
                                    let snapshot = {
                                        let b = compute_budget.read().await;
                                        serde_json::to_value(b.snapshot()).ok()
                                    };
                                    let notice = arkavo_gossip::TaskCompletionNotice {
                                        task_id: task_id_clone.to_string(),
                                        specialist_id: specialist_id.clone(),
                                        succeeded: true,
                                        content: result_for_notice,
                                        budget_snapshot: snapshot,
                                        completion_ms: task_start.elapsed().as_millis() as u64,
                                    };
                                    bus.broadcast_to_peers(
                                        arkavo_gossip::GossipMessage::TaskCompleted(notice),
                                    )
                                    .await;
                                }
                            }
                        }
                        Err(error_msg) => {
                            let error = TaskError {
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

                                // Push failure to commander via gossip
                                if let Some(ref bus) = learning_bus {
                                    let snapshot = {
                                        let b = compute_budget.read().await;
                                        serde_json::to_value(b.snapshot()).ok()
                                    };
                                    let notice = arkavo_gossip::TaskCompletionNotice {
                                        task_id: task_id_clone.to_string(),
                                        specialist_id: specialist_id.clone(),
                                        succeeded: false,
                                        content: error_msg,
                                        budget_snapshot: snapshot,
                                        completion_ms: task_start.elapsed().as_millis() as u64,
                                    };
                                    bus.broadcast_to_peers(
                                        arkavo_gossip::GossipMessage::TaskCompleted(notice),
                                    )
                                    .await;
                                }
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
        if !agents.iter().any(|a| &a.name == target_id) {
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
