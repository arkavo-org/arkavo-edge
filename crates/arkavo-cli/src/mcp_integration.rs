use arkavo_test::mcp::server::Tool as McpTool;
use arkavo_test::mcp::{
    device_tools::DeviceManagementKit,
    ios_tools::{ScreenCaptureKit, UiInteractionKit, UiQueryKit},
    simulator_tools::SimulatorControl,
    device_manager::DeviceManager,
};
use serde_json::Value;
use std::sync::Arc;
use std::collections::HashMap;
use tokio::runtime::Runtime;

#[derive(Debug, Clone)]
pub enum McpConnection {
    InProcess(InProcessMcp),
    External(crate::mcp_client::McpClient),
}

#[derive(Clone)]
pub struct InProcessMcp {
    tools: Arc<HashMap<String, Box<dyn McpTool>>>,
    runtime: Arc<Runtime>,
}

// Re-export Tool from mcp_client for compatibility
pub use crate::mcp_client::Tool;

impl McpConnection {
    pub fn new_in_process() -> Result<Self, Box<dyn std::error::Error>> {
        // Create tokio runtime for async operations
        let runtime = Arc::new(Runtime::new()?);
        
        // Create tools with shared device manager
        let device_manager = Arc::new(DeviceManager::new());
        
        let mut tools: HashMap<String, Box<dyn McpTool>> = HashMap::new();
        
        // Register all tools
        let simulator_control = SimulatorControl::new();
        tools.insert(simulator_control.schema().name.clone(), Box::new(simulator_control));
        
        let device_mgmt = DeviceManagementKit::new(device_manager.clone());
        tools.insert(device_mgmt.schema().name.clone(), Box::new(device_mgmt));
        
        let screen_capture = ScreenCaptureKit::new(device_manager.clone());
        tools.insert(screen_capture.schema().name.clone(), Box::new(screen_capture));
        
        let ui_interaction = UiInteractionKit::new(device_manager.clone());
        tools.insert(ui_interaction.schema().name.clone(), Box::new(ui_interaction));
        
        let ui_query = UiQueryKit::new(device_manager.clone());
        tools.insert(ui_query.schema().name.clone(), Box::new(ui_query));
        
        Ok(McpConnection::InProcess(InProcessMcp { 
            tools: Arc::new(tools),
            runtime,
        }))
    }
    
    pub fn new_external(mcp_url: Option<String>) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(McpConnection::External(crate::mcp_client::McpClient::new(mcp_url)?))
    }
    
    pub fn list_tools(&self) -> Result<Vec<Tool>, Box<dyn std::error::Error>> {
        match self {
            McpConnection::InProcess(mcp) => {
                let tools: Vec<Tool> = mcp.tools.values().map(|tool| {
                    let schema = tool.schema();
                    Tool {
                        name: schema.name.clone(),
                        description: schema.description.clone(),
                        input_schema: schema.parameters.clone(),
                    }
                }).collect();
                Ok(tools)
            }
            McpConnection::External(client) => {
                client.list_tools()
            }
        }
    }
    
    pub fn call_tool(&self, tool_name: &str, args: Value, _llm_origin: &str) -> Result<Value, Box<dyn std::error::Error>> {
        match self {
            McpConnection::InProcess(mcp) => {
                // Create a oneshot channel to get the result  
                let (tx, rx) = std::sync::mpsc::channel::<Result<Value, String>>();
                
                // Clone what we need
                let tools = mcp.tools.clone();
                let tool_name = tool_name.to_string();
                let runtime = mcp.runtime.clone();
                
                // Spawn a thread to run the async operation
                std::thread::spawn(move || {
                    let result = runtime.block_on(async move {
                        if let Some(tool) = tools.get(&tool_name) {
                            tool.execute(args).await
                                .map_err(|e| format!("Tool execution error: {}", e))
                        } else {
                            Err(format!("Tool '{}' not found", tool_name))
                        }
                    });
                    tx.send(result).ok();
                });
                
                // Wait for the result
                rx.recv()
                    .map_err(|_| "Failed to receive tool result")?
                    .map_err(|e: String| e.into())
            }
            McpConnection::External(client) => {
                client.call_tool(tool_name, args, _llm_origin)
            }
        }
    }
}

// Re-export McpClient for backward compatibility  
pub use crate::mcp_client::McpClient;

// Implement Debug manually for InProcessMcp
impl std::fmt::Debug for InProcessMcp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InProcessMcp")
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .field("runtime", &"Runtime")
            .finish()
    }
}