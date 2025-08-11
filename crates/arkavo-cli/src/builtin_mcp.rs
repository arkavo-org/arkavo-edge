use arkavo_mcp_core::Tool;
use arkavo_mcp_runtime::tools::{EchoTool, HealthTool, OllamaConfigTool};
use arkavo_protocol::mcp_registry::{McpConnectionTrait, Tool as RegistryTool};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Built-in MCP connection that provides default tools
pub struct BuiltinMcpConnection {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl BuiltinMcpConnection {
    pub fn new() -> Self {
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();

        // Register built-in tools that don't require external dependencies
        let echo_tool = Arc::new(EchoTool::new());
        tools.insert("echo".to_string(), echo_tool);

        let health_tool = Arc::new(HealthTool::new());
        tools.insert("health".to_string(), health_tool);

        let ollama_config = Arc::new(OllamaConfigTool::new());
        tools.insert("ollama_config".to_string(), ollama_config);

        // Note: UiControlTool and UiInspectTool require UI channels and should be
        // registered separately when a UI is available

        Self { tools }
    }
}

impl Default for BuiltinMcpConnection {
    fn default() -> Self {
        Self::new()
    }
}

impl McpConnectionTrait for BuiltinMcpConnection {
    fn list_tools(&self) -> Result<Vec<RegistryTool>, Box<dyn std::error::Error>> {
        let mut tools = Vec::new();

        for (name, tool) in &self.tools {
            let schema = tool.schema();
            tools.push(RegistryTool {
                name: name.clone(),
                description: schema.description.clone(),
                input_schema: Some(schema.parameters.clone()),
            });
        }

        Ok(tools)
    }

    fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
        _llm_provider: &str,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        if let Some(tool) = self.tools.get(tool_name) {
            // Use blocking to execute the async tool
            #[allow(clippy::disallowed_methods)]
            let handle = tokio::runtime::Handle::current();
            #[allow(clippy::disallowed_methods)]
            let result = handle.block_on(tool.execute(arguments))?;
            Ok(result)
        } else {
            Err(format!("Tool '{tool_name}' not found").into())
        }
    }
}
