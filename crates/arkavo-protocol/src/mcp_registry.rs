use arkavo_mcp_core::ToolSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::error;

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
}

impl McpRegistry {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
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

    /// Get list of connected servers and their status
    pub async fn get_server_status(&self) -> HashMap<String, String> {
        let mut status = HashMap::new();

        {
            let connections = self.connections.read().await;
            for (name, _connection) in connections.iter() {
                // TODO: Add actual connection health check
                status.insert(name.clone(), "connected".to_string());
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
                    description: tool.description,
                    parameters: tool.input_schema.unwrap_or_default(),
                })
                .collect();
            Ok(schemas)
        } else {
            Err(format!("MCP server '{server_name}' not found").into())
        }
    }
}

impl Default for McpRegistry {
    fn default() -> Self {
        Self::new()
    }
}
