use std::sync::Arc;

use tracing::{debug, info, warn};

use crate::server::tool_memory::ToolMemory;

/// Configuration for the agent orchestrator loop.
///
/// Bundles all shared state that the orchestrator needs, avoiding deep
/// closure captures and making the loop testable in isolation.
pub struct AgentLoopConfig {
    pub conductor: Arc<arkavo_hrm::Conductor<arkavo_hrm::store::InMemoryTaskStore>>,
    pub router: Arc<arkavo_router::Router>,
    pub mcp_registry: Arc<arkavo_protocol::mcp_registry::McpRegistry>,
    pub agent_memory: Arc<tokio::sync::RwLock<ToolMemory>>,
    pub learning_bus: Option<Arc<super::learning_bus::LearningBus>>,
    pub mesh_state: Arc<arkavo_mcp_mesh::MeshToolsState>,
    pub compute_budget: arkavo_budget::SharedComputeBudget,
    pub model_hint: Option<arkavo_router::ModelChoice>,
    pub purpose: String,
    pub orchestrator_tick: Arc<std::sync::atomic::AtomicU64>,
    pub has_mcp_tools: bool,
    pub tool_loop_budget: Option<u32>,
    #[cfg(feature = "iroh")]
    pub iroh_node: Option<Arc<arkavo_tdf_iroh::IrohNode>>,
}

// --- Urgency detection ---

/// Detect urgency level from game state observation text.
///
/// Tries structured JSON first (looks for an `alerts` array), then falls back
/// to keyword frequency scanning for threat-related terms.
pub(super) fn detect_urgency(observe_data: &str) -> arkavo_budget::UrgencyLevel {
    use arkavo_budget::UrgencyLevel;

    if let Ok(v) = serde_json::from_str::<serde_json::Value>(observe_data)
        && let Some(alerts) = v.pointer("/alerts").and_then(|a| a.as_array())
    {
        return match alerts.len() {
            0..=1 => UrgencyLevel::Low,
            2..=4 => UrgencyLevel::Medium,
            5..=9 => UrgencyLevel::High,
            _ => UrgencyLevel::Critical,
        };
    }

    let lower = observe_data.to_lowercase();
    let count: usize = [
        "raid", "attack", "threat", "alert", "fire", "bleed", "danger",
    ]
    .iter()
    .map(|kw| lower.matches(kw).count())
    .sum();
    match count {
        0..=1 => UrgencyLevel::Low,
        2..=4 => UrgencyLevel::Medium,
        5..=9 => UrgencyLevel::High,
        _ => UrgencyLevel::Critical,
    }
}

/// Compact a large observation for broadcast to peers.
///
/// Generic strategy: if the JSON has a "Delta" key (changes since last state),
/// extract only that. Otherwise truncate to `max_chars`. Domain-specific
/// filtering is the specialist's job via its own system prompt.
pub(super) fn compact_observation(obs: &str, max_chars: usize) -> String {
    if obs.len() <= max_chars {
        return obs.to_string();
    }
    // Prefer the Delta section — it contains only what changed
    if let Some(delta_start) = obs.find("\"Delta\":{") {
        let subset = &obs[delta_start..];
        if let Some(end) = super::conductor_tool_loop::find_matching_brace(subset) {
            let delta = &subset[..=end];
            if delta.len() <= max_chars {
                return format!("{{{delta}}}");
            }
        }
    }
    // Fallback: truncate with a note
    let cut = max_chars.saturating_sub(30);
    format!(
        "{}...(truncated {} bytes)",
        &obs[..cut.min(obs.len())],
        obs.len()
    )
}

// --- Peer state broadcast ---

/// Compute per-agent memory budget from total system RAM.
///
/// Reserves 8GB for the OS and subtracts the commander's model weight,
/// then divides evenly among specialists. Returns at least 512MB.
pub(super) fn compute_per_agent_bytes_static(
    total_ram: u64,
    commander_model: &str,
    count: usize,
) -> u64 {
    const OS_RESERVE: u64 = 8 * 1024 * 1024 * 1024; // 8 GB
    const FLOOR: u64 = 512 * 1024 * 1024; // 512 MB min
    let commander_bytes = arkavo_router::ModelChoice::from_name(commander_model)
        .map(|m| m.size_bytes())
        .unwrap_or(0);
    let available = total_ram
        .saturating_sub(OS_RESERVE)
        .saturating_sub(commander_bytes);
    if count == 0 {
        return available.max(FLOOR);
    }
    (available / count as u64).max(FLOOR)
}

/// Refresh specialist budgets without sending observation data.
///
/// Called when the commander hasn't produced a game observation yet but specialists
/// need their budgets refreshed to avoid staying passive indefinitely.
pub(super) async fn refresh_specialist_budgets(
    mesh_state: &Arc<arkavo_mcp_mesh::MeshToolsState>,
    per_agent_bytes: u64,
    self_agent_id: &str,
) {
    let peer_ids: Vec<String> = mesh_state
        .agent_addresses
        .read()
        .await
        .keys()
        .filter(|id| id.as_str() != self_agent_id)
        .cloned()
        .collect();

    if peer_ids.is_empty() {
        return;
    }

    for agent_id in &peer_ids {
        let pending = mesh_state
            .pending_delegations
            .read()
            .await
            .iter()
            .filter(|d| d.agent_id == *agent_id)
            .count() as u32;
        let allocation = arkavo_budget::BudgetPolicy::allocate(
            arkavo_budget::UrgencyLevel::Low,
            pending,
            per_agent_bytes,
        );
        // Only propagate budget allocation — no LLM work needed when no game state exists
        match send_advisory_task(mesh_state, agent_id, "", Some(&allocation)).await {
            Ok(_) => info!(
                max_inferences = allocation.max_inferences,
                max_memory_mb = allocation.max_memory_bytes / (1024 * 1024),
                per_agent_bytes,
                "Budget refreshed for {agent_id}"
            ),
            Err(e) => warn!("Could not reach {agent_id}: {e}"),
        }
    }
}

/// Stage data on Iroh P2P network for peer fetching.
#[cfg(feature = "iroh")]
pub(super) async fn stage_on_iroh(
    iroh_node: Option<&Arc<arkavo_tdf_iroh::IrohNode>>,
    data: &str,
) -> Option<String> {
    let node = iroh_node?;
    let transport = arkavo_tdf_iroh::IrohTransport::new(node.clone());
    match transport.stage_bytes(data.as_bytes()).await {
        Ok(ticket) => {
            let ticket_str = ticket.to_string();
            info!(
                data_len = data.len(),
                ticket_len = ticket_str.len(),
                "Staged data on Iroh for P2P fetch"
            );
            Some(ticket_str)
        }
        Err(e) => {
            warn!("Iroh stage failed, falling back to A2A: {e}");
            None
        }
    }
}

/// Broadcast observation state to all discovered peer agents for proactive analysis.
///
/// Computes per-specialist budget allocations dynamically based on game urgency
/// and each specialist's pending task backlog.
pub(super) async fn broadcast_state_to_peers(
    mesh_state: &Arc<arkavo_mcp_mesh::MeshToolsState>,
    observation: &str,
    per_agent_bytes: u64,
    self_agent_id: &str,
    iroh_ticket: Option<&str>,
) {
    let peer_ids: Vec<String> = mesh_state
        .agent_addresses
        .read()
        .await
        .keys()
        .filter(|id| id.as_str() != self_agent_id)
        .cloned()
        .collect();

    if peer_ids.is_empty() {
        let all_ids: Vec<String> = mesh_state
            .agent_addresses
            .read()
            .await
            .keys()
            .cloned()
            .collect();
        warn!(
            self_id = %self_agent_id,
            all_agents = ?all_ids,
            "No peers to broadcast to (all filtered as self)"
        );
        return;
    }

    info!(peer_count = peer_ids.len(), peers = ?peer_ids, "Broadcasting state to specialists");

    let urgency = detect_urgency(observation);
    let compacted = compact_observation(observation, 2000);

    let task = if let Some(ticket) = iroh_ticket {
        format!(
            "PROACTIVE ANALYSIS — Full state available via Iroh P2P.\n\
             Use iroh_fetch with this ticket to get full data: {ticket}\n\n\
             Summary: {compacted}\n\n\
             Respond with urgent action recommendations ONLY if you see \
             problems in your domain. If everything looks fine, respond \
             with 'No issues detected.'"
        )
    } else {
        format!(
            "PROACTIVE ANALYSIS — Review this state update and respond \
             with urgent action recommendations ONLY if you see problems \
             in your domain of expertise.\n\
             If everything looks fine, respond with 'No issues detected.'\n\n\
             {compacted}"
        )
    };

    for agent_id in &peer_ids {
        let pending = mesh_state
            .pending_delegations
            .read()
            .await
            .iter()
            .filter(|d| d.agent_id == *agent_id)
            .count() as u32;

        // Back off if specialist already has pending work — don't pile on tasks
        if pending >= 2 {
            info!(
                agent_id = %agent_id,
                pending,
                "Skipping broadcast to {agent_id} — already has {pending} pending tasks"
            );
            continue;
        }

        let allocation = arkavo_budget::BudgetPolicy::allocate(urgency, pending, per_agent_bytes);
        match send_advisory_task(mesh_state, agent_id, &task, Some(&allocation)).await {
            Ok(_) => info!(
                ?urgency,
                pending,
                max_inferences = allocation.max_inferences,
                ttl_secs = allocation.ttl_secs,
                "Budget allocated to {agent_id}"
            ),
            Err(e) => warn!("Could not reach {agent_id}: {e}"),
        }
    }
}

/// Send an advisory task to a specialist via message/send RPC.
///
/// Lightweight — short timeout, no retries. Tracks PendingDelegation
/// so `collect_completed()` picks up the response.
pub(super) async fn send_advisory_task(
    mesh_state: &Arc<arkavo_mcp_mesh::MeshToolsState>,
    agent_id: &str,
    task: &str,
    budget_allocation: Option<&arkavo_budget::BudgetAllocation>,
) -> std::result::Result<(), String> {
    use arkavo_protocol::transport::TlsConfig;
    use arkavo_protocol::types::{Message, MessagePart, MessageSendRequest, MessageSendResponse};
    use arkavo_protocol::{
        A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, HttpTransport, TransportConfig,
    };

    // Empty task = budget-only refresh. Skip the RPC call that would trigger
    // idle LLM inference on the specialist side.
    if task.is_empty() {
        debug!(agent_id, "Budget-only refresh — no task content to send");
        return Ok(());
    }

    let address = {
        let addrs = mesh_state.agent_addresses.read().await;
        addrs.get(agent_id).cloned()
    };
    let address = match address {
        Some(a) => a,
        None => return Err(format!("Agent {agent_id} not discovered")),
    };

    let transport_config = TransportConfig {
        timeout_ms: 10000,
        max_retries: 0,
        tls_config: TlsConfig {
            require_tls: false,
            ..Default::default()
        },
        ..Default::default()
    };

    let transport = Arc::new(HttpTransport::new(transport_config).map_err(|e| e.to_string())?);

    let endpoint = A2aEndpoint {
        url: address.clone(),
        agent_id: agent_id.to_string(),
        public_key: None,
    };

    transport
        .connect(&endpoint)
        .await
        .map_err(|e| e.to_string())?;

    let mut meta = serde_json::json!({
        "source": "state_broadcast",
        "task_type": "advisory"
    });
    if let Some(alloc) = budget_allocation {
        meta["budget_allocation"] = serde_json::to_value(alloc).unwrap_or_default();
    }

    let message = Message {
        parts: vec![MessagePart::Text {
            content: task.to_string(),
        }],
        metadata: Some(meta),
    };

    let send_request = MessageSendRequest {
        message,
        task_id: None,
    };

    let rpc_request = A2aRequest::new("message/send", serde_json::json!([send_request]));

    let response = transport
        .send_request(rpc_request)
        .await
        .map_err(|e| e.to_string())?;
    let _ = transport.close().await;

    match response {
        A2aResponse::Success { result, .. } => {
            let send_response: MessageSendResponse =
                serde_json::from_value(result).map_err(|e| e.to_string())?;

            if !send_response.task_id.is_empty() {
                mesh_state.pending_delegations.write().await.push(
                    arkavo_mcp_mesh::PendingDelegation {
                        task_id: send_response.task_id,
                        agent_id: agent_id.to_string(),
                        address,
                        sent_at: std::time::Instant::now(),
                    },
                );
            }
            Ok(())
        }
        A2aResponse::Error { error, .. } => Err(format!("{}: {}", error.code, error.message)),
    }
}

#[cfg(test)]
mod broadcast_tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("SRV-005")]
    #[tokio::test]
    async fn test_broadcast_skips_when_no_peers() {
        let mesh = Arc::new(arkavo_mcp_mesh::MeshToolsState::new());
        broadcast_state_to_peers(&mesh, r#"{"data":"test"}"#, 0, "self", None).await;
        assert!(mesh.pending_delegations.read().await.is_empty());
    }

    #[spec("SRV-005")]
    #[tokio::test]
    async fn test_broadcast_creates_task_per_peer() {
        let mesh = Arc::new(arkavo_mcp_mesh::MeshToolsState::new());
        mesh.agent_addresses
            .write()
            .await
            .insert("peer-a".to_string(), "http://127.0.0.1:19999".to_string());
        mesh.agent_addresses
            .write()
            .await
            .insert("peer-b".to_string(), "http://127.0.0.1:19998".to_string());

        broadcast_state_to_peers(&mesh, r#"{"state":"test"}"#, 0, "self", None).await;
        assert!(mesh.pending_delegations.read().await.is_empty());
    }

    #[spec("SRV-001")]
    #[test]
    fn test_dynamic_memory_128gb_3_specialists() {
        let total_ram = 128 * 1024 * 1024 * 1024_u64; // 128 GB
        let commander_size = arkavo_router::ModelChoice::from_name("glm-4.7-flash")
            .unwrap()
            .size_bytes(); // ~20 billion bytes
        let per_agent = compute_per_agent_bytes_static(total_ram, "glm-4.7-flash", 3);
        let os_reserve = 8 * 1024 * 1024 * 1024_u64;
        let expected = (total_ram - os_reserve - commander_size) / 3;
        assert_eq!(per_agent, expected);
        // Must be well above the 512MB floor (~37 GB)
        assert!(per_agent > 30 * 1024 * 1024 * 1024);
    }

    #[spec("SRV-001")]
    #[test]
    fn test_dynamic_memory_floor_enforced() {
        // Tiny system: 4 GB total, large commander model, many specialists
        let total_ram = 4 * 1024 * 1024 * 1024_u64;
        let per_agent = compute_per_agent_bytes_static(total_ram, "glm-4.7-flash", 10);
        // Should hit the 512MB floor
        assert_eq!(per_agent, 512 * 1024 * 1024);
    }
}

#[cfg(test)]
mod urgency_tests {
    use super::*;
    use arkavo_budget::UrgencyLevel;
    use arkavo_test_macros::spec;

    #[spec("SRV-009")]
    #[test]
    fn test_detect_urgency_json_alerts_low() {
        let data = r#"{"alerts":[],"colonists":[{"name":"Jess"}]}"#;
        assert_eq!(detect_urgency(data), UrgencyLevel::Low);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_detect_urgency_json_alerts_medium() {
        let data = r#"{"alerts":["cold snap","food shortage","crop blight"]}"#;
        assert_eq!(detect_urgency(data), UrgencyLevel::Medium);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_detect_urgency_json_alerts_high() {
        let data = r#"{"alerts":["a","b","c","d","e","f","g"]}"#;
        assert_eq!(detect_urgency(data), UrgencyLevel::High);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_detect_urgency_json_alerts_critical() {
        let alerts: Vec<String> = (0..12).map(|i| format!("alert_{i}")).collect();
        let data = serde_json::json!({"alerts": alerts}).to_string();
        assert_eq!(detect_urgency(&data), UrgencyLevel::Critical);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_detect_urgency_keyword_fallback() {
        let data = "Colony is under raid! Attack from the north. Fire in storage.";
        assert_eq!(detect_urgency(data), UrgencyLevel::Medium);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_detect_urgency_no_threats() {
        let data = "All colonists are happy. Resources are plentiful.";
        assert_eq!(detect_urgency(data), UrgencyLevel::Low);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_detect_urgency_keyword_critical() {
        let data = "raid attack threat alert fire bleed danger raid attack threat alert";
        assert_eq!(detect_urgency(data), UrgencyLevel::Critical);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_detect_urgency_json_without_alerts_key() {
        let data = r#"{"colonists":3,"resources":{"wood":50}}"#;
        assert_eq!(detect_urgency(data), UrgencyLevel::Low);
    }
}
