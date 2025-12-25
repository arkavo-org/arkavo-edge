use crate::mcp_client::{JsonRpcNotification, McpClient};
use serde_json::Value;
use tokio::sync::broadcast;

#[cfg(all(target_os = "macos", feature = "mcp-tools"))]
use {
    crate::memory_integration::MemoryIntegration, arkavo_mcp_macos::mcp::mcp_connection as base,
    tokio::runtime::Handle,
};

#[cfg(all(unix, feature = "mcp-tools"))]
use arkavo_mcp_tools::mcp_connection as tools;

// Re-export Tool from mcp_client for compatibility
pub use crate::mcp_client::Tool;

#[derive(Debug, Clone)]
pub enum McpConnection {
    #[cfg(all(target_os = "macos", feature = "mcp-tools"))]
    InProcess(base::McpConnection),
    #[cfg(all(unix, feature = "mcp-tools"))]
    CrossPlatform(tools::McpConnection),
    External(McpClient),
}

impl McpConnection {
    #[cfg(all(target_os = "macos", feature = "mcp-tools"))]
    #[allow(clippy::disallowed_methods)]
    pub fn new_in_process() -> Result<Self, Box<dyn std::error::Error>> {
        // Check if we're in a runtime to determine how to initialize memory
        let memory_integration = match Handle::try_current() {
            Ok(handle) => {
                // We're in an async context, use the current handle
                handle.block_on(async { MemoryIntegration::new().await })?
            }
            Err(_) => {
                // Not in a runtime, this is an error in MCP context
                // MCP servers should always be running in an async context
                return Err("MCP server must be run within a tokio runtime".into());
            }
        };

        // Get memory tools
        let additional_tools = memory_integration.get_tools();

        // Create the base MCP connection with additional tools
        let base_connection =
            base::McpConnection::new_in_process_with_additional_tools(additional_tools)?;

        Ok(Self::InProcess(base_connection))
    }

    #[cfg(all(target_os = "macos", feature = "mcp-tools"))]
    pub async fn new_in_process_async() -> Result<Self, Box<dyn std::error::Error>> {
        // Initialize memory asynchronously
        let memory_integration = MemoryIntegration::new().await?;

        // Get memory tools
        let additional_tools = memory_integration.get_tools();

        // Create the base MCP connection with additional tools
        let base_connection =
            base::McpConnection::new_in_process_with_additional_tools(additional_tools)?;

        Ok(Self::InProcess(base_connection))
    }

    #[cfg(all(unix, feature = "mcp-tools"))]
    pub fn new_cross_platform() -> Result<Self, Box<dyn std::error::Error>> {
        // Create cross-platform MCP connection with git, filesystem, and code analysis tools
        let connection = tools::McpConnection::new()?;
        Ok(Self::CrossPlatform(connection))
    }

    pub fn new_external(mcp_url: Option<String>) -> Result<Self, Box<dyn std::error::Error>> {
        if let Some(url) = mcp_url {
            let client = McpClient::new(Some(url))?;
            Ok(Self::External(client))
        } else {
            Err("MCP URL not provided for external connection".into())
        }
    }

    pub fn list_tools(&self) -> Result<Vec<Tool>, Box<dyn std::error::Error>> {
        match self {
            #[cfg(all(target_os = "macos", feature = "mcp-tools"))]
            Self::InProcess(base_conn) => {
                let tool_names = base_conn.list_tools();
                let tools = tool_names
                    .into_iter()
                    .filter_map(|name| {
                        base_conn.get_tool_schema(&name).map(|schema| Tool {
                            name: name.clone(),
                            description: schema
                                .get("description")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            input_schema: schema,
                        })
                    })
                    .collect();
                Ok(tools)
            }
            #[cfg(all(unix, feature = "mcp-tools"))]
            Self::CrossPlatform(tools_conn) => {
                let tool_names = tools_conn.list_tools();
                let tools = tool_names
                    .into_iter()
                    .filter_map(|name| {
                        tools_conn.get_tool_schema(&name).map(|schema| Tool {
                            name: name.clone(),
                            description: schema
                                .get("description")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            input_schema: schema,
                        })
                    })
                    .collect();
                Ok(tools)
            }
            Self::External(client) => client.list_tools(),
        }
    }

    pub fn call_tool(
        &self,
        tool_name: &str,
        args: Value,
        llm_origin: &str,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        match self {
            #[cfg(all(target_os = "macos", feature = "mcp-tools"))]
            Self::InProcess(base_conn) => base_conn
                .call_tool(tool_name, args, llm_origin)
                .map_err(|e| e.into()),
            #[cfg(all(unix, feature = "mcp-tools"))]
            Self::CrossPlatform(tools_conn) => tools_conn
                .call_tool(tool_name, args, llm_origin)
                .map_err(|e| e.into()),
            Self::External(client) => client.call_tool(tool_name, args, llm_origin),
        }
    }

    /// Subscribe to push notifications from this connection
    /// Returns Some(receiver) for External connections, None for others
    pub fn subscribe_notifications(&self) -> Option<broadcast::Receiver<JsonRpcNotification>> {
        match self {
            Self::External(client) => Some(client.subscribe_notifications()),
            #[cfg(all(target_os = "macos", feature = "mcp-tools"))]
            Self::InProcess(_) => None,
            #[cfg(all(unix, feature = "mcp-tools"))]
            Self::CrossPlatform(_) => None,
        }
    }
}

/// Implement the McpClient trait from arkavo-mcp-tools for McpConnection
/// This allows McpConnection to be used with ToolRegistry::from_mcp_connection()
#[cfg(all(unix, feature = "mcp-tools"))]
impl arkavo_mcp_tools::McpClient for McpConnection {
    fn list_tools(&self) -> Result<Vec<arkavo_mcp_tools::McpTool>, Box<dyn std::error::Error>> {
        let protocol_tools = McpConnection::list_tools(self)?;
        Ok(protocol_tools
            .into_iter()
            .map(|t| arkavo_mcp_tools::McpTool {
                name: t.name,
                description: t.description,
                input_schema: Some(t.input_schema),
            })
            .collect())
    }

    fn call_tool(
        &self,
        tool_name: &str,
        args: Value,
        llm_origin: &str,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        McpConnection::call_tool(self, tool_name, args, llm_origin)
    }
}
