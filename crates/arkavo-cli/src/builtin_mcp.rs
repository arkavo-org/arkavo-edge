use arkavo_mcp::{McpClient, McpTool, Tool};
use arkavo_mcp_runtime::tools::{HealthTool, OllamaConfigTool};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(all(target_os = "macos", feature = "mcp-macos"))]
use tracing::{error, info};

/// Built-in MCP connection that provides default tools
pub struct BuiltinMcpConnection {
    tools: HashMap<String, Arc<dyn Tool>>,
    // Optional delegate for test tools
    #[cfg(all(target_os = "macos", feature = "mcp-macos"))]
    test_connection: Option<crate::mcp_integration::McpConnection>,
}

impl BuiltinMcpConnection {
    pub fn new() -> Self {
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();

        // Register built-in tools that don't require external dependencies
        let health_tool = Arc::new(HealthTool::new());
        tools.insert("health".to_string(), health_tool);

        let ollama_config = Arc::new(OllamaConfigTool::new());
        tools.insert("ollama_config".to_string(), ollama_config);

        // Note: UiControlTool and UiInspectTool require UI channels and should be
        // registered separately when a UI is available

        // Note: Test tools initialization is deferred to avoid runtime conflicts
        // They will be initialized lazily when needed
        Self {
            tools,
            #[cfg(all(target_os = "macos", feature = "mcp-macos"))]
            test_connection: None,
        }
    }

    #[cfg(all(target_os = "macos", feature = "mcp-macos"))]
    pub async fn new_with_test_tools() -> Self {
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();

        // Register built-in tools that don't require external dependencies
        let health_tool = Arc::new(HealthTool::new());
        tools.insert("health".to_string(), health_tool);

        let ollama_config = Arc::new(OllamaConfigTool::new());
        tools.insert("ollama_config".to_string(), ollama_config);

        // Try to initialize test tools if available
        let test_connection =
            match crate::mcp_integration::McpConnection::new_in_process_async().await {
                Ok(conn) => {
                    info!("Registered MCP test tools from arkavo-mcp-macos");
                    Some(conn)
                }
                Err(e) => {
                    error!("Could not initialize MCP test tools: {e}");
                    None
                }
            };

        Self {
            tools,
            test_connection,
        }
    }

    #[cfg(not(all(target_os = "macos", feature = "mcp-macos")))]
    pub fn new_with_test_tools() -> Self {
        // When test harness is not available, just use the regular new() method
        Self::new()
    }
}

impl Default for BuiltinMcpConnection {
    fn default() -> Self {
        Self::new()
    }
}

impl McpClient for BuiltinMcpConnection {
    fn list_tools(&self) -> Result<Vec<McpTool>, Box<dyn std::error::Error + Send + Sync>> {
        let mut tools = Vec::new();

        // Add built-in runtime tools
        for (name, tool) in &self.tools {
            let schema = tool.schema();
            tools.push(McpTool {
                name: name.clone(),
                description: schema.description.clone(),
                input_schema: Some(schema.parameters.clone()),
            });
        }

        // Add test tools if available
        #[cfg(all(target_os = "macos", feature = "mcp-macos"))]
        if let Some(ref test_conn) = self.test_connection {
            match test_conn.list_tools() {
                Ok(test_tools) => {
                    for tool in test_tools {
                        tools.push(McpTool {
                            name: tool.name,
                            description: tool.description,
                            input_schema: Some(tool.input_schema),
                        });
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Could not list test tools: {e}");
                }
            }
        }

        Ok(tools)
    }

    fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
        #[allow(unused_variables)] llm_provider: &str,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        // First check built-in tools
        if let Some(tool) = self.tools.get(tool_name) {
            // Use blocking to execute the async tool
            #[allow(clippy::disallowed_methods)]
            let handle = tokio::runtime::Handle::current();
            #[allow(clippy::disallowed_methods)]
            let result = handle.block_on(tool.execute(arguments)).map_err(
                |e| -> Box<dyn std::error::Error + Send + Sync> {
                    Box::new(std::io::Error::other(e.to_string()))
                },
            )?;
            return Ok(result);
        }

        // Then check test tools if available
        #[cfg(all(target_os = "macos", feature = "mcp-macos"))]
        {
            if let Some(ref test_conn) = self.test_connection {
                return test_conn
                    .call_tool(tool_name, arguments, llm_provider)
                    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                        Box::new(std::io::Error::other(e.to_string()))
                    });
            }
        }

        Err(format!("Tool '{tool_name}' not found").into())
    }
}
