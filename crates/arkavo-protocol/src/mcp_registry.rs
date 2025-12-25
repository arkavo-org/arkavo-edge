use crate::types::{AgentCard, AgentStatus};
use arkavo_mcp::{McpClient, McpTool, ToolSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};
use tracing::{debug, error, info};

/// MCP notification from server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpNotification {
    pub server: String,
    pub method: String,
    pub params: Option<Value>,
}

/// Type alias for backward compatibility
pub type Tool = McpTool;

/// Trait alias for MCP connections (re-export from arkavo_mcp)
pub trait McpConnectionTrait: McpClient {}
impl<T: McpClient> McpConnectionTrait for T {}

/// Registered connection with cached tools
struct RegisteredConnection {
    connection: Box<dyn McpConnectionTrait>,
    cached_tools: Vec<Tool>,
    /// Parameter aliases: tool_name -> (alternative_param -> canonical_param)
    param_aliases: HashMap<String, HashMap<String, String>>,
}

/// Registry to manage multiple MCP server connections
pub struct McpRegistry {
    connections: Arc<RwLock<HashMap<String, RegisteredConnection>>>,
    agents: Arc<RwLock<HashMap<String, AgentCard>>>,
    agent_status: Arc<RwLock<HashMap<String, AgentStatus>>>,
    notification_tx: broadcast::Sender<McpNotification>,
}

impl McpRegistry {
    pub fn new() -> Self {
        let (notification_tx, _) = broadcast::channel(256);
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
            agents: Arc::new(RwLock::new(HashMap::new())),
            agent_status: Arc::new(RwLock::new(HashMap::new())),
            notification_tx,
        }
    }

    /// Subscribe to notifications from all MCP servers
    pub fn subscribe_notifications(&self) -> broadcast::Receiver<McpNotification> {
        self.notification_tx.subscribe()
    }

    /// Send a notification to all subscribers (used by MCP clients)
    pub fn emit_notification(&self, notification: McpNotification) {
        let _ = self.notification_tx.send(notification);
    }

    /// Register a new MCP connection (caches tools at registration time)
    pub async fn register(&self, name: String, connection: Box<dyn McpConnectionTrait>) {
        // Cache tools at registration time to avoid repeated tools/list calls
        let cached_tools = match connection.list_tools() {
            Ok(tools) => {
                info!(
                    server = %name,
                    count = tools.len(),
                    "Cached tools from MCP server"
                );
                tools
            }
            Err(e) => {
                error!(server = %name, error = %e, "Failed to cache tools from MCP server");
                Vec::new()
            }
        };

        let mut connections = self.connections.write().await;
        connections.insert(
            name,
            RegisteredConnection {
                connection,
                cached_tools,
                param_aliases: HashMap::new(),
            },
        );
    }

    /// Refresh cached tools for a specific server
    pub async fn refresh_tools(
        &self,
        server_name: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut connections = self.connections.write().await;
        if let Some(registered) = connections.get_mut(server_name) {
            let tools = registered.connection.list_tools()?;
            info!(
                server = %server_name,
                count = tools.len(),
                "Refreshed cached tools"
            );
            registered.cached_tools = tools;
            Ok(())
        } else {
            Err(format!("MCP server '{server_name}' not found").into())
        }
    }

    /// List all available tools from all connections (uses cached tools)
    pub async fn list_all_tools(&self) -> Result<Vec<Tool>, Box<dyn std::error::Error + Send + Sync>> {
        let mut all_tools = Vec::new();

        {
            let connections = self.connections.read().await;
            debug!("Listing tools from {} MCP connection(s)", connections.len());

            for (server_name, registered) in connections.iter() {
                debug!(
                    server = %server_name,
                    count = registered.cached_tools.len(),
                    "Using cached tools from MCP server"
                );
                // Prefix tool names with server name to avoid conflicts
                for tool in &registered.cached_tools {
                    let mut prefixed_tool = tool.clone();
                    prefixed_tool.name = format!("{server_name}:{}", tool.name);
                    debug!(tool = %prefixed_tool.name, "Registered tool");
                    all_tools.push(prefixed_tool);
                }
            }
        }

        debug!("Total tools available: {}", all_tools.len());
        Ok(all_tools)
    }

    /// Execute a tool on the appropriate MCP server
    ///
    /// Tool names can be either:
    /// - Prefixed: "server:tool-name" - routes to specific server
    /// - Unprefixed: "tool-name" - searches all servers for matching tool
    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
        llm_provider: &str,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        debug!(
            tool = %tool_name,
            provider = %llm_provider,
            "Routing tool call"
        );

        let connections = self.connections.read().await;

        // Check if tool name has server prefix (format: "server:tool")
        if let Some((server_name, actual_tool_name)) = tool_name.split_once(':') {
            // Prefixed format - route to specific server
            if let Some(registered) = connections.get(server_name) {
                // Apply parameter aliases before executing
                let normalized_args = Self::apply_param_aliases(
                    &registered.param_aliases,
                    actual_tool_name,
                    arguments,
                );

                debug!(
                    server = %server_name,
                    tool = %actual_tool_name,
                    args = %serde_json::to_string(&normalized_args).unwrap_or_default(),
                    "Executing tool on MCP server"
                );

                return Self::execute_tool(
                    registered.connection.as_ref(),
                    server_name,
                    actual_tool_name,
                    normalized_args,
                    llm_provider,
                );
            }
            error!(server = %server_name, "MCP server not found");
            return Err(format!("MCP server '{server_name}' not found").into());
        }

        // Unprefixed format - search all servers for matching tool
        for (server_name, registered) in connections.iter() {
            if registered.cached_tools.iter().any(|t| t.name == tool_name) {
                // Apply parameter aliases before executing
                let normalized_args =
                    Self::apply_param_aliases(&registered.param_aliases, tool_name, arguments);

                debug!(
                    server = %server_name,
                    tool = %tool_name,
                    args = %serde_json::to_string(&normalized_args).unwrap_or_default(),
                    "Found tool in server"
                );

                return Self::execute_tool(
                    registered.connection.as_ref(),
                    server_name,
                    tool_name,
                    normalized_args,
                    llm_provider,
                );
            }
        }

        error!(tool = %tool_name, "Tool not found in any MCP server");
        Err(format!("Tool '{tool_name}' not found in any MCP server").into())
    }

    /// Execute a tool on a specific connection
    fn execute_tool(
        connection: &dyn McpConnectionTrait,
        server_name: &str,
        tool_name: &str,
        arguments: Value,
        llm_provider: &str,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        match connection.call_tool(tool_name, arguments, llm_provider) {
            Ok(result) => {
                debug!(
                    server = %server_name,
                    tool = %tool_name,
                    result = %serde_json::to_string(&result).unwrap_or_default(),
                    "Tool execution succeeded"
                );
                Ok(result)
            }
            Err(e) => {
                error!(
                    server = %server_name,
                    tool = %tool_name,
                    error = %e,
                    "Tool execution failed"
                );
                Err(e)
            }
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

    /// Get list of connected servers and their status (uses cached tool count)
    pub async fn get_server_status(&self) -> HashMap<String, String> {
        let mut status = HashMap::new();

        {
            let connections = self.connections.read().await;
            for (name, registered) in connections.iter() {
                // Report cached tool count - no network call needed
                let health_status =
                    format!("connected ({} tools cached)", registered.cached_tools.len());
                status.insert(name.clone(), health_status);
            }
        }

        status
    }

    /// Disconnect and remove a server
    pub async fn disconnect(&self, server_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.connections.write().await.remove(server_name);
        Ok(())
    }

    /// Get tool schemas for a specific server (uses cached tools)
    pub async fn get_tool_schemas(
        &self,
        server_name: &str,
    ) -> Result<Vec<ToolSchema>, Box<dyn std::error::Error + Send + Sync>> {
        let connections = self.connections.read().await;

        if let Some(registered) = connections.get(server_name) {
            // Convert cached Tools to ToolSchemas
            let schemas: Vec<ToolSchema> = registered
                .cached_tools
                .iter()
                .map(|tool| ToolSchema {
                    name: tool.name.clone(),
                    aliases: None,
                    description: tool.description.clone(),
                    parameters: tool.input_schema.clone().unwrap_or_default(),
                })
                .collect();
            Ok(schemas)
        } else {
            Err(format!("MCP server '{server_name}' not found").into())
        }
    }

    /// Set parameter aliases for tools on a specific server
    ///
    /// Aliases map alternative parameter names to canonical ones.
    /// Example: For find-block tool, "block" -> "blockType"
    pub async fn set_param_aliases(
        &self,
        server_name: &str,
        aliases: HashMap<String, HashMap<String, String>>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut connections = self.connections.write().await;

        if let Some(registered) = connections.get_mut(server_name) {
            let alias_count: usize = aliases.values().map(|m| m.len()).sum();
            info!(
                server = %server_name,
                tools = aliases.len(),
                aliases = alias_count,
                "Set parameter aliases for MCP server"
            );
            registered.param_aliases = aliases;
            Ok(())
        } else {
            Err(format!("MCP server '{server_name}' not found").into())
        }
    }

    /// Check if a server has parameter aliases configured
    pub async fn has_param_aliases(&self, server_name: &str) -> bool {
        let connections = self.connections.read().await;
        connections
            .get(server_name)
            .map(|r| !r.param_aliases.is_empty())
            .unwrap_or(false)
    }

    /// Apply parameter aliases to tool arguments
    fn apply_param_aliases(
        aliases: &HashMap<String, HashMap<String, String>>,
        tool_name: &str,
        mut arguments: Value,
    ) -> Value {
        if let Some(tool_aliases) = aliases.get(tool_name)
            && let Some(obj) = arguments.as_object_mut()
        {
            // Collect keys to rename (avoid mutating while iterating)
            let renames: Vec<(String, String, Value)> = obj
                .iter()
                .filter_map(|(key, value)| {
                    tool_aliases
                        .get(key)
                        .map(|canonical| (key.clone(), canonical.clone(), value.clone()))
                })
                .collect();

            for (alt_key, canonical_key, value) in renames {
                if !obj.contains_key(&canonical_key) {
                    obj.remove(&alt_key);
                    obj.insert(canonical_key.clone(), value);
                    debug!(
                        tool = %tool_name,
                        from = %alt_key,
                        to = %canonical_key,
                        "Applied parameter alias"
                    );
                }
            }
        }
        arguments
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
