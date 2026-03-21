//! MCP tools for agent mesh orchestration
//!
//! These tools enable an AI orchestrator agent to discover, query, and delegate
//! tasks to specialist agents in the Arkavo mesh network.
//!
//! # Tools
//!
//! - `list_agents` - List all discovered agents on the mesh network
//! - `agent_query` - Find agents by capability or purpose
//! - `send_task` - Delegate a task to a specific agent
//! - `get_task_status` - Check the status of a delegated task
//!
//! # Usage
//!
//! ```ignore
//! use arkavo_mcp_mesh::{MeshToolsState, register_tools};
//! use arkavo_mcp_tools::ToolRegistry;
//! use std::sync::Arc;
//!
//! let state = Arc::new(MeshToolsState::new());
//! let mut registry = ToolRegistry::new();
//! register_tools(&mut registry, state);
//! ```

// Re-export types from arkavo-mcp-tools
pub use arkavo_mcp_tools::{Result, Tool, ToolError, ToolRegistry, ToolSchema};

use arkavo_protocol::agent_registry::AgentRegistry;
use arkavo_protocol::transport::TlsConfig;
use arkavo_protocol::types::{
    Message, MessagePart, MessageSendRequest, MessageSendResponse, TaskGetRequest, TaskGetResponse,
    TaskStatus,
};
use arkavo_protocol::{
    A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, HttpTransport, TransportConfig,
};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

// Re-export error type for mesh-specific errors
pub use arkavo_mcp_tools::ToolError as MeshToolError;

/// A specialist task that was delegated via `send_task` and is awaiting response.
#[derive(Debug, Clone)]
pub struct PendingDelegation {
    pub task_id: String,
    pub agent_id: String,
    pub address: String,
    pub sent_at: std::time::Instant,
}

/// A specialist delegation whose response has been fetched.
#[derive(Debug, Clone)]
pub struct CompletedDelegation {
    pub agent_id: String,
    pub response: String,
    pub response_latency_ms: u64,
    /// Budget snapshot from the specialist (if available)
    pub budget_snapshot: Option<arkavo_budget::ComputeBudgetSnapshot>,
}

/// Shared state for mesh tools
pub struct MeshToolsState {
    /// Registry of discovered agents
    pub registry: Arc<AgentRegistry>,
    /// Cache of agent addresses discovered via mDNS
    pub agent_addresses: Arc<RwLock<HashMap<String, String>>>,
    /// Delegated tasks awaiting specialist responses
    pub pending_delegations: Arc<RwLock<Vec<PendingDelegation>>>,
    /// Completions pushed by gossip (specialist → commander, no polling)
    push_completions: Arc<RwLock<Vec<CompletedDelegation>>>,
}

impl MeshToolsState {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(AgentRegistry::new()),
            agent_addresses: Arc::new(RwLock::new(HashMap::new())),
            pending_delegations: Arc::new(RwLock::new(Vec::new())),
            push_completions: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Push a completion received via gossip into the local queue.
    /// Called by the gossip handler when a TaskCompleted message arrives.
    pub async fn push_completed(&self, task_id: &str, completion: CompletedDelegation) {
        // Remove matching pending delegation so polling doesn't duplicate it
        let mut pending = self.pending_delegations.write().await;
        pending.retain(|d| d.task_id != task_id);
        drop(pending);

        self.push_completions.write().await.push(completion);
    }

    /// Fetch all completed specialist responses, removing them from pending.
    ///
    /// Drains push-delivered completions first (zero network cost), then falls
    /// back to HTTP polling for any remaining pending delegations.
    pub async fn collect_completed(&self) -> Vec<CompletedDelegation> {
        // Drain push-delivered completions first (from gossip TaskCompleted messages)
        let mut pushed: Vec<CompletedDelegation> =
            self.push_completions.write().await.drain(..).collect();

        let mut pending = self.pending_delegations.write().await;
        if pending.is_empty() {
            return pushed;
        }

        let mut completed = Vec::new();
        let mut still_pending = Vec::new();

        for delegation in pending.drain(..) {
            // Expire delegations older than 5 minutes
            if delegation.sent_at.elapsed() > Duration::from_secs(300) {
                tracing::warn!(
                    agent_id = %delegation.agent_id,
                    task_id = %delegation.task_id,
                    "Delegation expired after 5 minutes"
                );
                continue;
            }

            match fetch_task_result(&delegation).await {
                Ok(Some(response)) => {
                    let latency_ms = delegation.sent_at.elapsed().as_millis() as u64;
                    // Fetch budget snapshot from specialist for feedback to commander
                    let budget_snapshot =
                        fetch_budget_snapshot(&delegation.address, &delegation.agent_id).await;
                    if let Some(ref snap) = budget_snapshot {
                        tracing::info!(
                            agent_id = %delegation.agent_id,
                            response_len = response.len(),
                            remaining_inferences = snap.remaining_inferences,
                            status = %snap.status,
                            "Specialist response pre-fetched with budget feedback"
                        );
                    } else {
                        tracing::info!(
                            agent_id = %delegation.agent_id,
                            response_len = response.len(),
                            "Specialist response pre-fetched (no budget data)"
                        );
                    }
                    completed.push(CompletedDelegation {
                        agent_id: delegation.agent_id,
                        response,
                        response_latency_ms: latency_ms,
                        budget_snapshot,
                    });
                }
                Ok(None) => {
                    // Not done yet — keep pending
                    still_pending.push(delegation);
                }
                Err(e) => {
                    tracing::warn!(
                        agent_id = %delegation.agent_id,
                        error = %e,
                        "Failed to fetch delegation status"
                    );
                    still_pending.push(delegation);
                }
            }
        }

        *pending = still_pending;
        pushed.extend(completed);
        pushed
    }
}

impl MeshToolsState {
    /// Trigger mDNS discovery to populate agent_addresses.
    ///
    /// Called by the orchestrator loop so broadcast/budget-refresh
    /// can reach specialists without waiting for the LLM to call send_task.
    pub async fn discover_peers(&self) {
        match discover_and_register_agents(self).await {
            Ok(()) => {
                let count = self.agent_addresses.read().await.len();
                tracing::info!(count, "Peer discovery complete");
            }
            Err(e) => {
                tracing::debug!("Peer discovery failed: {e}");
            }
        }
    }
}

impl Default for MeshToolsState {
    fn default() -> Self {
        Self::new()
    }
}

/// Tool to list all discovered agents on the mesh
pub struct ListAgentsTool {
    schema: ToolSchema,
    state: Arc<MeshToolsState>,
}

impl ListAgentsTool {
    pub fn new(state: Arc<MeshToolsState>) -> Self {
        Self {
            schema: ToolSchema {
                name: "list_agents".to_string(),
                aliases: Some(vec!["mesh_agents".to_string()]),
                description: "List all agents discovered on the Arkavo mesh network. Returns agent IDs, names, purposes, capabilities, and current load.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "refresh": {
                            "type": "boolean",
                            "description": "If true, perform fresh mDNS discovery. Default: false (use cached agents)"
                        }
                    },
                    "required": []
                }),
            },
            state,
        }
    }
}

#[async_trait]
impl Tool for ListAgentsTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let refresh = args
            .get("refresh")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if refresh {
            // Perform mDNS discovery
            if let Err(e) = discover_and_register_agents(&self.state).await {
                return Ok(json!({
                    "success": false,
                    "error": format!("Failed to discover agents: {e}")
                }));
            }
        }

        let agents = self.state.registry.get_all_agents().await;
        let addresses = self.state.agent_addresses.read().await;

        let agent_list: Vec<Value> = agents
            .iter()
            .map(|agent| {
                json!({
                    "agent_id": agent.agent_id,
                    "name": agent.name,
                    "purpose": agent.purpose,
                    "capabilities": agent.capabilities,
                    "load": format!("{:.0}%", agent.load * 100.0),
                    "is_available": agent.is_available,
                    "address": addresses.get(&agent.agent_id),
                    "last_seen": agent.last_seen.to_rfc3339()
                })
            })
            .collect();

        Ok(json!({
            "success": true,
            "agent_count": agent_list.len(),
            "agents": agent_list
        }))
    }
}

/// Tool to query agents by capability
pub struct AgentQueryTool {
    schema: ToolSchema,
    state: Arc<MeshToolsState>,
}

impl AgentQueryTool {
    pub fn new(state: Arc<MeshToolsState>) -> Self {
        Self {
            schema: ToolSchema {
                name: "agent_query".to_string(),
                aliases: Some(vec!["find_agent".to_string()]),
                description: "Find agents that match specific capabilities or purpose. Returns matching agents sorted by relevance.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "capability": {
                            "type": "string",
                            "description": "Required capability (e.g., 'security_analysis', 'code_review', 'testing')"
                        },
                        "purpose_contains": {
                            "type": "string",
                            "description": "Text to match in agent's purpose description"
                        }
                    },
                    "required": []
                }),
            },
            state,
        }
    }
}

#[async_trait]
impl Tool for AgentQueryTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let capability = args.get("capability").and_then(|v| v.as_str());
        let purpose_contains = args.get("purpose_contains").and_then(|v| v.as_str());

        if capability.is_none() && purpose_contains.is_none() {
            return Ok(json!({
                "success": false,
                "error": "At least one of 'capability' or 'purpose_contains' must be provided"
            }));
        }

        let all_agents = self.state.registry.get_all_agents().await;

        // Build address lookup map (scope the lock)
        let address_map: std::collections::HashMap<String, String> = {
            let addresses = self.state.agent_addresses.read().await;
            addresses.clone()
        };

        let mut matching_agents = Vec::new();
        for agent in all_agents {
            let cap_matches =
                capability.is_none_or(|cap| agent.capabilities.iter().any(|c| c.contains(cap)));

            let purpose_matches = purpose_contains.is_none_or(|purpose_text| {
                agent
                    .purpose
                    .to_lowercase()
                    .contains(&purpose_text.to_lowercase())
            });

            if cap_matches && purpose_matches {
                matching_agents.push(json!({
                    "agent_id": agent.agent_id,
                    "name": agent.name,
                    "purpose": agent.purpose,
                    "capabilities": agent.capabilities,
                    "load": format!("{:.0}%", agent.load * 100.0),
                    "is_available": agent.is_available,
                    "address": address_map.get(&agent.agent_id)
                }));
            }
        }

        // Sort by load (prefer less busy agents)
        matching_agents.sort_by(|a, b| {
            let load_a = a["load"].as_str().unwrap_or("100%");
            let load_b = b["load"].as_str().unwrap_or("100%");
            load_a.cmp(load_b)
        });

        Ok(json!({
            "success": true,
            "match_count": matching_agents.len(),
            "agents": matching_agents,
            "query": {
                "capability": capability,
                "purpose_contains": purpose_contains
            }
        }))
    }
}

/// Tool to send a task to a specific agent
pub struct SendTaskTool {
    schema: ToolSchema,
    state: Arc<MeshToolsState>,
}

impl SendTaskTool {
    pub fn new(state: Arc<MeshToolsState>) -> Self {
        Self {
            schema: ToolSchema {
                name: "send_task".to_string(),
                aliases: Some(vec!["delegate_task".to_string()]),
                description: "Send a task to a specific agent for execution. Returns a task_id for monitoring progress.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "agent_id": {
                            "type": "string",
                            "description": "ID of the agent to send the task to"
                        },
                        "task": {
                            "type": "string",
                            "description": "Task description to send to the agent"
                        },
                        "metadata": {
                            "type": "object",
                            "description": "Optional metadata to include with the task"
                        }
                    },
                    "required": ["agent_id", "task"]
                }),
            },
            state,
        }
    }

    /// Resolve an agent ID to its address, auto-discovering peers if needed.
    /// Handles typos via fuzzy matching (edit distance ≤ 2).
    async fn resolve_agent_address(&self, agent_id: &str) -> crate::Result<String> {
        // First try with current known agents
        if let Some(addr) = self.try_find_address(agent_id).await {
            return Ok(addr);
        }

        // No match — trigger mDNS discovery and retry
        tracing::info!("Agent '{agent_id}' not found, triggering auto-discovery");
        let _ = discover_and_register_agents(&self.state).await;

        // Retry after discovery
        if let Some(addr) = self.try_find_address(agent_id).await {
            return Ok(addr);
        }

        let available: Vec<_> = self
            .state
            .agent_addresses
            .read()
            .await
            .keys()
            .cloned()
            .collect();
        Err(MeshToolError::Execution(format!(
            "Agent '{agent_id}' not found after discovery. Available: {available:?}"
        )))
    }

    async fn try_find_address(&self, agent_id: &str) -> Option<String> {
        let addresses = self.state.agent_addresses.read().await;
        if let Some(addr) = addresses.get(agent_id) {
            return Some(addr.clone());
        }
        // Fuzzy match: edit distance ≤ 2 handles common LLM typos
        addresses
            .iter()
            .find(|(id, _)| levenshtein_close(id, agent_id))
            .map(|(matched_id, addr)| {
                tracing::info!(
                    requested = agent_id,
                    matched = %matched_id,
                    "Fuzzy-matched agent ID"
                );
                addr.clone()
            })
    }
}

#[async_trait]
impl Tool for SendTaskTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let agent_id = args
            .get("agent_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| MeshToolError::InvalidParams("agent_id is required".to_string()))?;

        let task = args
            .get("task")
            .and_then(|v| v.as_str())
            .ok_or_else(|| MeshToolError::InvalidParams("task is required".to_string()))?;

        let metadata = args.get("metadata").cloned();

        // Get agent address — auto-discover if needed, fuzzy match on typos
        let address = self.resolve_agent_address(agent_id).await?;

        // Create transport
        let transport_config = TransportConfig {
            timeout_ms: 60000,
            max_retries: 2,
            tls_config: TlsConfig {
                require_tls: false,
                ..Default::default()
            },
            ..Default::default()
        };

        let transport =
            Arc::new(HttpTransport::new(transport_config).map_err(|e| {
                MeshToolError::Execution(format!("Failed to create transport: {e}"))
            })?);

        let endpoint = A2aEndpoint {
            url: address.clone(),
            agent_id: agent_id.to_string(),
            public_key: None,
        };

        // Connect
        transport
            .connect(&endpoint)
            .await
            .map_err(|e| MeshToolError::Execution(format!("Failed to connect to agent: {e}")))?;

        // Build message with budget allocation for specialist compute control
        let default_budget = arkavo_budget::BudgetAllocation::default();
        let mut final_metadata = metadata.unwrap_or_else(|| {
            json!({
                "source": "orchestrator",
                "task_type": "delegated"
            })
        });
        if final_metadata.get("budget_allocation").is_none() {
            final_metadata["budget_allocation"] =
                serde_json::to_value(&default_budget).unwrap_or_default();
        }

        let message = Message {
            parts: vec![MessagePart::Text {
                content: task.to_string(),
            }],
            metadata: Some(final_metadata),
        };

        let send_request = MessageSendRequest {
            message,
            task_id: None,
        };

        // Send task
        let rpc_request = A2aRequest::new("message/send", json!([send_request]));

        let response = transport
            .send_request(rpc_request)
            .await
            .map_err(|e| MeshToolError::Execution(format!("Failed to send task: {e}")))?;

        let _ = transport.close().await;

        match response {
            A2aResponse::Success { result, .. } => {
                let send_response: MessageSendResponse =
                    serde_json::from_value(result).map_err(|e| {
                        MeshToolError::Execution(format!("Failed to parse response: {e}"))
                    })?;

                // Track delegation so orchestrator can pre-fetch the response
                if !send_response.task_id.is_empty() {
                    self.state
                        .pending_delegations
                        .write()
                        .await
                        .push(PendingDelegation {
                            task_id: send_response.task_id.clone(),
                            agent_id: agent_id.to_string(),
                            address: address.clone(),
                            sent_at: std::time::Instant::now(),
                        });
                }

                Ok(json!({
                    "success": true,
                    "task_id": send_response.task_id,
                    "status": format!("{:?}", send_response.status),
                    "agent_id": agent_id,
                    "agent_address": address
                }))
            }
            A2aResponse::Error { error, .. } => Ok(json!({
                "success": false,
                "error": format!("{}: {}", error.code, error.message)
            })),
        }
    }
}

/// Tool to get the status of a task
pub struct GetTaskStatusTool {
    schema: ToolSchema,
    state: Arc<MeshToolsState>,
}

impl GetTaskStatusTool {
    pub fn new(state: Arc<MeshToolsState>) -> Self {
        Self {
            schema: ToolSchema {
                name: "get_task_status".to_string(),
                aliases: Some(vec!["task_status".to_string()]),
                description: "Get the current status of a task that was sent to an agent."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "agent_id": {
                            "type": "string",
                            "description": "ID of the agent the task was sent to"
                        },
                        "task_id": {
                            "type": "string",
                            "description": "ID of the task to check"
                        }
                    },
                    "required": ["agent_id", "task_id"]
                }),
            },
            state,
        }
    }
}

#[async_trait]
impl Tool for GetTaskStatusTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let agent_id = args
            .get("agent_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| MeshToolError::InvalidParams("agent_id is required".to_string()))?;

        let task_id = args
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| MeshToolError::InvalidParams("task_id is required".to_string()))?;

        // Get agent address
        let addresses = self.state.agent_addresses.read().await;
        let address = addresses
            .get(agent_id)
            .ok_or_else(|| MeshToolError::Execution(format!("Agent '{agent_id}' not found")))?
            .clone();
        drop(addresses);

        // Create transport
        let transport_config = TransportConfig {
            timeout_ms: 30000,
            max_retries: 1,
            tls_config: TlsConfig {
                require_tls: false,
                ..Default::default()
            },
            ..Default::default()
        };

        let transport =
            Arc::new(HttpTransport::new(transport_config).map_err(|e| {
                MeshToolError::Execution(format!("Failed to create transport: {e}"))
            })?);

        let endpoint = A2aEndpoint {
            url: address.clone(),
            agent_id: agent_id.to_string(),
            public_key: None,
        };

        transport
            .connect(&endpoint)
            .await
            .map_err(|e| MeshToolError::Execution(format!("Failed to connect to agent: {e}")))?;

        // Get task status
        let get_request = TaskGetRequest {
            task_id: task_id.to_string(),
        };

        let rpc_request = A2aRequest::new("tasks/get", json!([get_request]));

        let response = transport
            .send_request(rpc_request)
            .await
            .map_err(|e| MeshToolError::Execution(format!("Failed to get task status: {e}")))?;

        let _ = transport.close().await;

        match response {
            A2aResponse::Success { result, .. } => {
                let task_response: TaskGetResponse =
                    serde_json::from_value(result).map_err(|e| {
                        MeshToolError::Execution(format!("Failed to parse response: {e}"))
                    })?;

                let mut result_json = json!({
                    "success": true,
                    "task_id": task_response.task_id,
                    "status": format!("{:?}", task_response.status),
                    "is_complete": matches!(task_response.status, TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Canceled)
                });

                if let Some(progress) = task_response.progress {
                    result_json["progress"] = json!({
                        "message": progress.message,
                        "percentage": progress.percentage
                    });
                }

                if let Some(error) = task_response.error {
                    result_json["error"] = json!({
                        "code": error.code,
                        "message": error.message
                    });
                }

                if let Some(result_msg) = task_response.result {
                    let content: Vec<String> = result_msg
                        .parts
                        .iter()
                        .filter_map(|p| {
                            if let MessagePart::Text { content } = p {
                                Some(content.clone())
                            } else {
                                None
                            }
                        })
                        .collect();
                    result_json["result"] = json!(content.join("\n"));
                }

                Ok(result_json)
            }
            A2aResponse::Error { error, .. } => Ok(json!({
                "success": false,
                "error": format!("{}: {}", error.code, error.message)
            })),
        }
    }
}

/// Fetch the result of a pending delegation from the specialist agent.
///
/// Returns `Ok(Some(response))` if the task is completed,
/// `Ok(None)` if still in progress, or `Err` on transport failure.
async fn fetch_task_result(
    delegation: &PendingDelegation,
) -> std::result::Result<Option<String>, String> {
    let transport_config = TransportConfig {
        timeout_ms: 5000, // Short timeout — just a status check
        max_retries: 0,
        tls_config: TlsConfig {
            require_tls: false,
            ..Default::default()
        },
        ..Default::default()
    };

    let transport = Arc::new(
        HttpTransport::new(transport_config)
            .map_err(|e| format!("Transport creation failed: {e}"))?,
    );

    let endpoint = A2aEndpoint {
        url: delegation.address.clone(),
        agent_id: delegation.agent_id.clone(),
        public_key: None,
    };

    transport
        .connect(&endpoint)
        .await
        .map_err(|e| format!("Connection failed: {e}"))?;

    let get_request = TaskGetRequest {
        task_id: delegation.task_id.clone(),
    };
    let rpc_request = A2aRequest::new("tasks/get", json!([get_request]));

    let response = transport
        .send_request(rpc_request)
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    let _ = transport.close().await;

    match response {
        A2aResponse::Success { result, .. } => {
            let task_response: TaskGetResponse =
                serde_json::from_value(result).map_err(|e| format!("Parse failed: {e}"))?;

            if matches!(
                task_response.status,
                TaskStatus::Completed | TaskStatus::Failed
            ) {
                let text = task_response
                    .result
                    .map(|msg| {
                        msg.parts
                            .iter()
                            .filter_map(|p| {
                                if let MessagePart::Text { content } = p {
                                    Some(content.clone())
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                Ok(Some(text))
            } else {
                Ok(None) // Still in progress
            }
        }
        A2aResponse::Error { error, .. } => Err(format!("{}: {}", error.code, error.message)),
    }
}

/// Fetch compute budget snapshot from a specialist agent via JSON-RPC.
async fn fetch_budget_snapshot(
    address: &str,
    agent_id: &str,
) -> Option<arkavo_budget::ComputeBudgetSnapshot> {
    let transport_config = TransportConfig {
        timeout_ms: 3000,
        max_retries: 0,
        tls_config: TlsConfig {
            require_tls: false,
            ..Default::default()
        },
        ..Default::default()
    };

    let transport = Arc::new(HttpTransport::new(transport_config).ok()?);
    let endpoint = A2aEndpoint {
        url: address.to_string(),
        agent_id: agent_id.to_string(),
        public_key: None,
    };

    transport.connect(&endpoint).await.ok()?;
    let rpc_request = A2aRequest::new("budget.compute_status", json!({}));
    let response = transport.send_request(rpc_request).await.ok()?;
    let _ = transport.close().await;

    match response {
        A2aResponse::Success { result, .. } => serde_json::from_value(result).ok(),
        _ => None,
    }
}

/// Discover agents via mDNS and register them
#[cfg(feature = "mdns")]
async fn discover_and_register_agents(
    state: &MeshToolsState,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use mdns_sd::{ServiceDaemon, ServiceEvent};

    let mdns = ServiceDaemon::new()?;
    let receiver = mdns.browse("_a2a._tcp.local.")?;

    let timeout = Duration::from_secs(3);
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => {
                if let ServiceEvent::ServiceResolved(info) = event {
                    let agent_id = info
                        .get_property_val_str("agent_id")
                        .unwrap_or("unknown")
                        .to_string();
                    let name = info.get_fullname().to_string();
                    let purpose = info
                        .get_property_val_str("purpose")
                        .unwrap_or("")
                        .to_string();

                    let capabilities_str = info
                        .get_property_val_str("capabilities")
                        .unwrap_or_default();
                    let mut capabilities: Vec<String> = if capabilities_str.is_empty() {
                        vec![]
                    } else {
                        capabilities_str.split(',').map(|s| s.to_string()).collect()
                    };

                    let mcp_tools_str = info.get_property_val_str("mcp_tools").unwrap_or_default();
                    if !mcp_tools_str.is_empty() {
                        let mcp_tools: Vec<String> =
                            mcp_tools_str.split(',').map(|s| s.to_string()).collect();
                        capabilities.extend(mcp_tools);
                    }

                    let address = info
                        .get_addresses()
                        .iter()
                        .next()
                        .map(|addr| format!("http://{}:{}", addr, info.get_port()));

                    if let Some(addr) = &address {
                        let mut addresses = state.agent_addresses.write().await;
                        addresses.insert(agent_id.clone(), addr.clone());
                    }

                    let metadata = HashMap::new();

                    let _ = state
                        .registry
                        .register_agent(
                            agent_id,
                            name,
                            purpose,
                            capabilities,
                            None,
                            metadata,
                            address,
                        )
                        .await;
                }
            }
            Err(_) => {
                // Timeout on recv - continue until overall timeout
            }
        }
    }

    mdns.shutdown().ok();
    Ok(())
}

#[cfg(not(feature = "mdns"))]
async fn discover_and_register_agents(
    _state: &MeshToolsState,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    Err("mDNS feature not enabled - cannot discover agents".into())
}

/// Check if two strings are within edit distance 2 (handles common LLM typos)
fn levenshtein_close(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (m, n) = (a.len(), b.len());
    if m.abs_diff(n) > 2 {
        return false;
    }
    let mut prev = (0..=n).collect::<Vec<_>>();
    let mut curr = vec![0; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n] <= 2
}

/// Register mesh orchestration tools with a ToolRegistry
///
/// This function follows the arkavo tool registration pattern where each crate
/// exports a `register_tools` function to avoid circular dependencies.
pub fn register_tools(registry: &mut ToolRegistry, state: Arc<MeshToolsState>) {
    registry.register(
        "list_agents",
        Box::new(ListAgentsTool::new(Arc::clone(&state))),
    );
    registry.register(
        "agent_query",
        Box::new(AgentQueryTool::new(Arc::clone(&state))),
    );
    registry.register("send_task", Box::new(SendTaskTool::new(Arc::clone(&state))));
    registry.register("get_task_status", Box::new(GetTaskStatusTool::new(state)));
}

/// Get all mesh orchestration tools with shared state
pub fn create_mesh_tools(state: Arc<MeshToolsState>) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ListAgentsTool::new(Arc::clone(&state))),
        Box::new(AgentQueryTool::new(Arc::clone(&state))),
        Box::new(SendTaskTool::new(Arc::clone(&state))),
        Box::new(GetTaskStatusTool::new(state)),
    ]
}

/// Get tool schemas for mesh orchestration tools
pub fn get_tool_schemas() -> Vec<ToolSchema> {
    let state = Arc::new(MeshToolsState::new());
    vec![
        ListAgentsTool::new(Arc::clone(&state)).schema,
        AgentQueryTool::new(Arc::clone(&state)).schema,
        SendTaskTool::new(Arc::clone(&state)).schema,
        GetTaskStatusTool::new(state).schema,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_list_agents_tool_schema() {
        let state = Arc::new(MeshToolsState::new());
        let tool = ListAgentsTool::new(state);
        let schema = tool.schema();

        assert_eq!(schema.name, "list_agents");
        assert!(schema.description.contains("mesh network"));
    }

    #[tokio::test]
    async fn test_agent_query_tool_schema() {
        let state = Arc::new(MeshToolsState::new());
        let tool = AgentQueryTool::new(state);
        let schema = tool.schema();

        assert_eq!(schema.name, "agent_query");
        assert!(schema.description.contains("capabilities"));
    }

    #[tokio::test]
    async fn test_send_task_tool_schema() {
        let state = Arc::new(MeshToolsState::new());
        let tool = SendTaskTool::new(state);
        let schema = tool.schema();

        assert_eq!(schema.name, "send_task");
        assert!(schema.description.contains("task_id"));
    }

    #[tokio::test]
    async fn test_get_task_status_tool_schema() {
        let state = Arc::new(MeshToolsState::new());
        let tool = GetTaskStatusTool::new(state);
        let schema = tool.schema();

        assert_eq!(schema.name, "get_task_status");
        assert!(schema.description.contains("status"));
    }
}
