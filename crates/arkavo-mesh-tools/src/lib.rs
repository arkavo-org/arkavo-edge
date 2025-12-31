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
//! use arkavo_mesh_tools::{MeshToolsState, register_tools};
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
    A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, ResilientTransport, TransportConfig,
};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

// Re-export error type for mesh-specific errors
pub use arkavo_mcp_tools::ToolError as MeshToolError;

/// Shared state for mesh tools
pub struct MeshToolsState {
    /// Registry of discovered agents
    pub registry: Arc<AgentRegistry>,
    /// Cache of agent addresses discovered via mDNS
    pub agent_addresses: Arc<RwLock<HashMap<String, String>>>,
}

impl MeshToolsState {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(AgentRegistry::new()),
            agent_addresses: Arc::new(RwLock::new(HashMap::new())),
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

        // Get agent address
        let addresses = self.state.agent_addresses.read().await;
        let address = addresses
            .get(agent_id)
            .ok_or_else(|| {
                MeshToolError::Execution(format!(
                    "Agent '{agent_id}' not found. Try list_agents with refresh=true first."
                ))
            })?
            .clone();
        drop(addresses);

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
            Arc::new(ResilientTransport::new(transport_config).map_err(|e| {
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

        // Build message
        let message = Message {
            parts: vec![MessagePart::Text {
                content: task.to_string(),
            }],
            metadata: metadata.or_else(|| {
                Some(json!({
                    "source": "orchestrator",
                    "task_type": "delegated"
                }))
            }),
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
            Arc::new(ResilientTransport::new(transport_config).map_err(|e| {
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

    Ok(())
}

#[cfg(not(feature = "mdns"))]
async fn discover_and_register_agents(
    _state: &MeshToolsState,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    Err("mDNS feature not enabled - cannot discover agents".into())
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
