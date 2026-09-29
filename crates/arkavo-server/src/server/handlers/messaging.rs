use arkavo_events::{Event, EventPayload, EventWriter};
use arkavo_hrm::{Conductor, store::InMemoryTaskStore};
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_protocol::metrics::{MetricsCollector, RpcTimer};
use arkavo_protocol::rate_limit::RateLimiter;
use arkavo_protocol::types::{
    AgentBroadcast, AgentQueryRequest, AgentQueryResponse, BroadcastType, MessagePart,
    MessageSendRequest, MessageSendResponse, TaskStatus,
};
use arkavo_tasks::task_executor::TaskExecutor;
use arkavo_tasks::task_store::TaskStore;
use jsonrpsee::types::ErrorObjectOwned;
use std::sync::Arc;
use tracing::{info, warn};

use super::super::LearningBus;
use super::super::agent_cycle_reply::{
    REQUEST_REPLY_BUDGET, apply_outcome, deliver_outcome_to_task,
};
use super::super::agent_event::CycleOutcome;
use super::super::config_helpers::AgentMetadata;
use super::super::pipeline::{self, Driving};
use super::super::tool_memory::ToolMemory;

#[cfg(test)]
// `#[tokio::test]` expands to `Runtime::block_on`, which the crate's lint set
// disallows in library code. Same waiver as the other async test modules here.
#[allow(clippy::disallowed_methods)]
mod answer_tests;
#[cfg(test)]
// Same waiver as above.
#[allow(clippy::disallowed_methods)]
mod budget_tests;
mod caller_budget;
pub(in crate::server) mod direct;
#[cfg(test)]
// Same waiver as above.
#[allow(clippy::disallowed_methods)]
mod pipeline_tests;
mod specialist;
#[cfg(test)]
// Same waiver as above.
#[allow(clippy::disallowed_methods)]
mod step_tests;
#[cfg(test)]
// Same waiver as above.
#[allow(clippy::disallowed_methods)]
mod test_agent;

use direct::DirectExecution;

/// Where the handler learns whether a message starts a pipeline run.
pub(in crate::server) enum PlanSource {
    /// The kit this process was started from, read when the message arrives
    /// so a kit that was edited since is followed as it stands.
    Kit,
    /// Decided by the caller, for a handler run without a kit on disk.
    #[cfg(test)]
    Decided(Driving),
}

impl PlanSource {
    async fn driving(&self, agent_metadata: &tokio::sync::RwLock<AgentMetadata>) -> Driving {
        match self {
            Self::Kit => {
                let role_id = agent_metadata.read().await.role_id.clone();
                pipeline::driving_from_kit(role_id.as_deref()).await
            }
            #[cfg(test)]
            Self::Decided(driving) => driving.clone(),
        }
    }
}

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
    agent_event_tx: Arc<
        tokio::sync::Mutex<
            Option<tokio::sync::mpsc::Sender<super::super::agent_event::AgentEvent>>,
        >,
    >,
    #[cfg(feature = "iroh")] iroh_node: Option<&Arc<arkavo_tdf_iroh::IrohNode>>,
    request: MessageSendRequest,
) -> Result<MessageSendResponse, ErrorObjectOwned> {
    handle_message_send_from(
        metrics,
        rate_limiter,
        task_executor,
        task_store,
        mcp_registry,
        conductor,
        router,
        learning_bus,
        budget_manager,
        model_hint,
        compute_budget,
        mesh_state,
        agent_metadata,
        agent_memory,
        agent_event_tx,
        #[cfg(feature = "iroh")]
        iroh_node,
        &PlanSource::Kit,
        request,
    )
    .await
}

/// [`handle_message_send`] with the pipeline decision taken from `plans`.
#[allow(clippy::too_many_arguments)]
pub(in crate::server) async fn handle_message_send_from(
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
    agent_event_tx: Arc<
        tokio::sync::Mutex<
            Option<tokio::sync::mpsc::Sender<super::super::agent_event::AgentEvent>>,
        >,
    >,
    #[cfg(feature = "iroh")] iroh_node: Option<&Arc<arkavo_tdf_iroh::IrohNode>>,
    plans: &PlanSource,
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

    // Refresh the compute budget from the allocation in the task metadata.
    // The allocation comes from the caller, so it is held to the budget this
    // agent was configured with.
    if let Some(allocation) = caller_budget::requested(request.message.metadata.as_ref()) {
        let mut budget = compute_budget.write().await;
        caller_budget::refresh_within_ceiling(&mut budget, &allocation).await;
        metrics.record_compute_budget_refresh();
        let snapshot = budget.snapshot();
        metrics.record_compute_budget_state(
            &snapshot.status,
            snapshot.remaining_inferences,
            snapshot.remaining_tokens,
        );
    }

    // Save metadata before submit_task consumes request.message
    let request_metadata_ref = request.message.metadata.clone();

    match task_executor.submit_task(request.message).await {
        Ok(task_id) => {
            if let Some(router) = router {
                let direct = DirectExecution {
                    router: router.clone(),
                    conductor: conductor.clone(),
                    mcp_registry: mcp_registry.clone(),
                    task_executor: task_executor.clone(),
                    learning_bus: learning_bus.cloned(),
                    compute_budget: compute_budget.clone(),
                    mesh_state: mesh_state.cloned(),
                    agent_metadata: agent_metadata.clone(),
                    model_hint,
                    #[cfg(feature = "iroh")]
                    iroh_node: iroh_node.cloned(),
                };

                // A step of someone else's run is answered and nothing more:
                // whatever this agent's kit says, a step never starts a run.
                let driving = if pipeline::is_marked(request_metadata_ref.as_ref()) {
                    None
                } else {
                    Some(plans.driving(agent_metadata).await)
                };
                match driving {
                    None => {
                        let allowed = pipeline::step_timeout(request_metadata_ref.as_ref());
                        tokio::spawn(pipeline::answer_step(
                            direct,
                            task_id,
                            task_content,
                            allowed,
                        ));
                    }
                    Some(Driving::Plan(plan)) => {
                        // The run owns this task from here to its end. No
                        // reply budget is set against it: a pipeline is
                        // bounded by the kit's own time budget, which the
                        // run enforces.
                        tokio::spawn(pipeline::drive_task(
                            direct,
                            *plan,
                            task_id,
                            task_content,
                            images,
                        ));
                    }
                    Some(Driving::Unknown(reason)) => {
                        apply_outcome(
                            task_executor,
                            &task_id,
                            &CycleOutcome::Failed { error: reason },
                        )
                        .await;
                    }
                    Some(Driving::No) => {
                        hand_to_agent(
                            direct,
                            task_store,
                            agent_memory,
                            &agent_event_tx,
                            request_metadata_ref.as_ref(),
                            task_id,
                            task_content,
                            images,
                        )
                        .await;
                    }
                }
            } else {
                // No router means no way to execute this task, ever. Submitting
                // it and walking away leaves the requester polling a task that
                // will never move.
                apply_outcome(
                    task_executor,
                    &task_id,
                    &CycleOutcome::Failed {
                        error: "agent has no router configured to execute this message".to_string(),
                    },
                )
                .await;
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

/// Hand a message to this agent as it runs: to its agent loop when it has
/// one, and to the conductor directly when it does not.
#[allow(clippy::too_many_arguments)]
async fn hand_to_agent(
    direct: DirectExecution,
    task_store: &Arc<dyn TaskStore>,
    agent_memory: &Arc<tokio::sync::RwLock<ToolMemory>>,
    agent_event_tx: &tokio::sync::Mutex<
        Option<tokio::sync::mpsc::Sender<super::super::agent_event::AgentEvent>>,
    >,
    metadata: Option<&serde_json::Value>,
    task_id: uuid::Uuid,
    task_content: String,
    images: Option<Vec<String>>,
) {
    if let Some(event_tx) = agent_event_tx.lock().await.clone() {
        // Orchestrator path: route through the agent event loop
        use super::super::agent_event::{AgentEvent, CorrelationId};

        let correlation_id = CorrelationId(uuid::Uuid::new_v4());
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let (outcome_tx, outcome_rx) = tokio::sync::oneshot::channel();

        let sender_did = metadata
            .and_then(|m| m.get("sender_did"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let _ = event_tx
            .send(AgentEvent::IncomingMessage {
                sender: sender_did,
                content: task_content,
                task_id,
                correlation_id,
                reply: reply_tx,
                outcome: outcome_tx,
            })
            .await;

        // The requester polls this task, so the cycle's answer — text, tool
        // summary, or refusal — has to land on it. The specialist path does
        // the same thing itself.
        let executor = direct.task_executor;
        tokio::spawn(async move {
            deliver_outcome_to_task(
                executor,
                task_id,
                reply_rx,
                outcome_rx,
                REQUEST_REPLY_BUDGET,
            )
            .await;
        });
    } else {
        // Specialist path: execute directly via conductor
        tokio::spawn(specialist::run(
            direct,
            task_store.clone(),
            agent_memory.clone(),
            task_id,
            task_content,
            images,
        ));
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
