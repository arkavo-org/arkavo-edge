use crate::types::{AgentCard, AgentStatus};
use arkavo_mcp_core::ToolSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

/// Tool information structure matching MCP protocol
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Option<Value>,
}

/// Trait for MCP connections that can be registered
pub trait McpConnectionTrait: Send + Sync {
    fn list_tools(&self) -> Result<Vec<Tool>, Box<dyn std::error::Error>>;
    fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
        llm_provider: &str,
    ) -> Result<Value, Box<dyn std::error::Error>>;
}

/// Registry to manage multiple MCP server connections
pub struct McpRegistry {
    connections: Arc<RwLock<HashMap<String, Box<dyn McpConnectionTrait>>>>,
    agents: Arc<RwLock<HashMap<String, AgentCard>>>,
    agent_status: Arc<RwLock<HashMap<String, AgentStatus>>>,
}

impl McpRegistry {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
            agents: Arc::new(RwLock::new(HashMap::new())),
            agent_status: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new MCP connection
    pub async fn register(&self, name: String, connection: Box<dyn McpConnectionTrait>) {
        let mut connections = self.connections.write().await;
        connections.insert(name, connection);
    }

    /// List all available tools from all connections
    pub async fn list_all_tools(&self) -> Result<Vec<Tool>, Box<dyn std::error::Error>> {
        let mut all_tools = Vec::new();

        {
            let connections = self.connections.read().await;
            for (server_name, connection) in connections.iter() {
                match connection.list_tools() {
                    Ok(tools) => {
                        // Prefix tool names with server name to avoid conflicts
                        for mut tool in tools {
                            tool.name = format!("{server_name}:{}", tool.name);
                            all_tools.push(tool);
                        }
                    }
                    Err(e) => {
                        error!(server = %server_name, error = %e, "Failed to list tools from MCP server");
                    }
                }
            }
        }

        Ok(all_tools)
    }

    /// Execute a tool on the appropriate MCP server
    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
        llm_provider: &str,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        // Parse server name from tool name (format: "server:tool")
        let parts: Vec<&str> = tool_name.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err("Tool name must be in format 'server:tool'".into());
        }

        let server_name = parts[0];
        let actual_tool_name = parts[1];

        let connections = self.connections.read().await;

        if let Some(connection) = connections.get(server_name) {
            connection.call_tool(actual_tool_name, arguments, llm_provider)
        } else {
            Err(format!("MCP server '{server_name}' not found").into())
        }
    }

    /// Clear all MCP connections (for hot-reload)
    pub async fn clear_connections(&self) {
        let mut connections = self.connections.write().await;
        let count = connections.len();
        connections.clear();
        if count > 0 {
            info!("Cleared {} MCP server connections for hot-reload", count);
        }
    }

    /// Remove a specific MCP connection
    pub async fn unregister(&self, name: &str) -> bool {
        let mut connections = self.connections.write().await;
        connections.remove(name).is_some()
    }

    /// Get list of connected servers and their status
    pub async fn get_server_status(&self) -> HashMap<String, String> {
        let mut status = HashMap::new();

        {
            let connections = self.connections.read().await;
            for (name, connection) in connections.iter() {
                // Perform health check by attempting to list tools
                let health_status = match connection.list_tools() {
                    Ok(tools) => format!("healthy ({} tools available)", tools.len()),
                    Err(e) => {
                        error!("Health check failed for {}: {}", name, e);
                        format!("unhealthy: {e}")
                    }
                };
                status.insert(name.clone(), health_status);
            }
        }

        status
    }

    /// Disconnect and remove a server
    pub async fn disconnect(&self, server_name: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.connections.write().await.remove(server_name);
        Ok(())
    }

    /// Get tool schemas for a specific server
    pub async fn get_tool_schemas(
        &self,
        server_name: &str,
    ) -> Result<Vec<ToolSchema>, Box<dyn std::error::Error>> {
        let connections = self.connections.read().await;

        if let Some(connection) = connections.get(server_name) {
            // For now, we'll convert Tools to ToolSchemas
            // In a real implementation, MCP should provide schemas directly
            let tools = connection.list_tools()?;
            let schemas: Vec<ToolSchema> = tools
                .into_iter()
                .map(|tool| ToolSchema {
                    name: tool.name,
                    aliases: None,
                    description: tool.description,
                    parameters: tool.input_schema.unwrap_or_default(),
                })
                .collect();
            Ok(schemas)
        } else {
            Err(format!("MCP server '{server_name}' not found").into())
        }
    }

    /// Register an agent with its card
    pub async fn register_agent(&self, agent_card: AgentCard) {
        let agent_id = agent_card.identity.id.clone();
        let mut agents = self.agents.write().await;
        agents.insert(agent_id.clone(), agent_card);

        // Set initial status
        let mut status = self.agent_status.write().await;
        status.insert(agent_id.clone(), AgentStatus::Online);

        info!("Registered agent: {}", agent_id);
    }

    /// Unregister an agent
    pub async fn unregister_agent(&self, agent_id: &str) {
        let mut agents = self.agents.write().await;
        agents.remove(agent_id);

        let mut status = self.agent_status.write().await;
        status.remove(agent_id);

        info!("Unregistered agent: {}", agent_id);
    }

    /// List all registered agents
    pub async fn list_agents(&self) -> Vec<AgentCard> {
        let agents = self.agents.read().await;
        agents.values().cloned().collect()
    }

    /// Update agent status
    pub async fn update_agent_status(&self, agent_id: &str, new_status: AgentStatus) {
        let mut status = self.agent_status.write().await;
        status.insert(agent_id.to_string(), new_status.clone());
        info!("Updated agent {} status to {:?}", agent_id, new_status);
    }

    /// Get agent status
    pub async fn get_agent_status(&self, agent_id: &str) -> Option<AgentStatus> {
        let status = self.agent_status.read().await;
        status.get(agent_id).cloned()
    }
}

impl Default for McpRegistry {
    fn default() -> Self {
        Self::new()
    }
}
