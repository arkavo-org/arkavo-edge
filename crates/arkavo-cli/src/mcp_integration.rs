use crate::mcp_client::McpClient;
use crate::memory_integration::MemoryIntegration;
use arkavo_test::mcp::mcp_connection as base;
use serde_json::Value;
use tokio::runtime::Handle;

// Re-export Tool from mcp_client for compatibility
pub use crate::mcp_client::Tool;

#[derive(Debug, Clone)]
pub enum McpConnection {
    InProcess(base::McpConnection),
    External(McpClient),
}

impl McpConnection {
    pub fn new_in_process() -> Result<Self, Box<dyn std::error::Error>> {
        // Initialize memory tools first
        eprintln!("Initializing memory tools...");

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
            Self::InProcess(base_conn) => base_conn
                .call_tool(tool_name, args, llm_origin)
                .map_err(|e| e.into()),
            Self::External(client) => client.call_tool(tool_name, args, llm_origin),
        }
    }
}
