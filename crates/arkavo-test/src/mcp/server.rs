use crate::{Result, TestError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::time::timeout;
use super::ios_tools::{UiInteractionKit, ScreenCaptureKit, UiQueryKit};

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolRequest {
    pub tool_name: String,
    pub params: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolResponse {
    pub tool_name: String,
    pub result: Value,
    pub success: bool,
}

pub struct McpTestServer {
    tools: Arc<RwLock<HashMap<String, Arc<dyn Tool>>>>,
    metrics: Arc<Metrics>,
}

impl std::fmt::Debug for McpTestServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpTestServer")
            .field("tools", &"<tools>")
            .field("metrics", &self.metrics)
            .finish()
    }
}

impl McpTestServer {
    pub fn new() -> Result<Self> {
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        
        tools.insert("query_state".to_string(), Arc::new(QueryStateKit::new()));
        tools.insert("mutate_state".to_string(), Arc::new(MutateStateKit::new()));
        tools.insert("snapshot".to_string(), Arc::new(SnapshotKit::new()));
        tools.insert("run_test".to_string(), Arc::new(RunTestKit::new()));
        
        // Add iOS-specific tools
        tools.insert("ui_interaction".to_string(), Arc::new(UiInteractionKit::new()));
        tools.insert("screen_capture".to_string(), Arc::new(ScreenCaptureKit::new()));
        tools.insert("ui_query".to_string(), Arc::new(UiQueryKit::new()));
        
        Ok(Self {
            tools: Arc::new(RwLock::new(tools)),
            metrics: Arc::new(Metrics::new()),
        })
    }
    
    pub fn register_tool(&self, name: String, tool: Arc<dyn Tool>) -> Result<()> {
        let mut tools = self.tools.write().map_err(|e| 
            TestError::Mcp(format!("Failed to acquire tool lock: {}", e)))?;
        tools.insert(name, tool);
        Ok(())
    }
    
    pub async fn call_tool(&self, request: ToolRequest) -> Result<ToolResponse> {
        if !self.is_allowed(&request.tool_name, &request.params) {
            return Err(TestError::Mcp("Tool not allowed".to_string()));
        }
        
        let result = timeout(
            Duration::from_secs(30),
            self.execute_tool(&request.tool_name, request.params)
        )
        .await
        .map_err(|_| TestError::Mcp("Tool execution timeout".to_string()))?
        ?;
        
        Ok(ToolResponse { 
            result,
            tool_name: request.tool_name,
            success: true,
        })
    }
    
    fn is_allowed(&self, tool_name: &str, _params: &Value) -> bool {
        matches!(
            tool_name,
            "query_state" | "mutate_state" | "snapshot" | "run_test" |
            "ui_interaction" | "screen_capture" | "ui_query"
        )
    }
    
    async fn execute_tool(&self, tool_name: &str, params: Value) -> Result<Value> {
        let tool = {
            let tools = self.tools.read().map_err(|e| 
                TestError::Mcp(format!("Failed to acquire tool lock: {}", e)))?;
            
            tools.get(tool_name)
                .ok_or_else(|| TestError::Mcp(format!("Tool not found: {}", tool_name)))?
                .clone()
        };
        
        tool.execute(params).await
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    async fn execute(&self, params: Value) -> Result<Value>;
    fn schema(&self) -> &ToolSchema;
}

#[derive(Debug, Clone)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

pub struct QueryStateKit {
    schema: ToolSchema,
}

impl QueryStateKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "query_state".to_string(),
                description: "Query application state".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "entity": {
                            "type": "string",
                            "description": "Entity to query"
                        },
                        "filter": {
                            "type": "object",
                            "description": "Optional filter criteria"
                        }
                    },
                    "required": ["entity"]
                }),
            },
        }
    }
}

impl Default for QueryStateKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for QueryStateKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let _entity = params.get("entity")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing entity parameter".to_string()))?;
        
        Ok(serde_json::json!({
            "state": "mocked_state",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    }
    
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct MutateStateKit {
    schema: ToolSchema,
}

impl MutateStateKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "mutate_state".to_string(),
                description: "Mutate application state".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "entity": {
                            "type": "string",
                            "description": "Entity to mutate"
                        },
                        "action": {
                            "type": "string",
                            "description": "Action to perform"
                        },
                        "data": {
                            "type": "object",
                            "description": "Data for the mutation"
                        }
                    },
                    "required": ["entity", "action"]
                }),
            },
        }
    }
}

impl Default for MutateStateKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MutateStateKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let _entity = params.get("entity")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing entity parameter".to_string()))?;
        
        let _action = params.get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;
        
        Ok(serde_json::json!({
            "success": true,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    }
    
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct SnapshotKit {
    schema: ToolSchema,
}

impl SnapshotKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "snapshot".to_string(),
                description: "Create or restore state snapshots".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["create", "restore", "list"],
                            "description": "Snapshot action"
                        },
                        "name": {
                            "type": "string",
                            "description": "Snapshot name"
                        }
                    },
                    "required": ["action"]
                }),
            },
        }
    }
}

impl Default for SnapshotKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SnapshotKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params.get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;
        
        match action {
            "create" => Ok(serde_json::json!({
                "snapshot_id": uuid::Uuid::new_v4().to_string(),
                "timestamp": chrono::Utc::now().to_rfc3339()
            })),
            "restore" => Ok(serde_json::json!({
                "success": true,
                "timestamp": chrono::Utc::now().to_rfc3339()
            })),
            "list" => Ok(serde_json::json!({
                "snapshots": []
            })),
            _ => Err(TestError::Mcp(format!("Invalid action: {}", action))),
        }
    }
    
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct RunTestKit {
    schema: ToolSchema,
}

impl RunTestKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "run_test".to_string(),
                description: "Execute a test scenario".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "test_name": {
                            "type": "string",
                            "description": "Name of the test to run"
                        },
                        "timeout": {
                            "type": "integer",
                            "description": "Timeout in seconds"
                        }
                    },
                    "required": ["test_name"]
                }),
            },
        }
    }
}

impl Default for RunTestKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for RunTestKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let test_name = params.get("test_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing test_name parameter".to_string()))?;
        
        Ok(serde_json::json!({
            "test_name": test_name,
            "status": "passed",
            "duration_ms": 150,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    }
    
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[derive(Debug)]
pub struct Metrics {
    tool_calls: Arc<RwLock<HashMap<String, u64>>>,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            tool_calls: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    pub fn record_tool_call(&self, tool_name: &str) {
        if let Ok(mut calls) = self.tool_calls.write() {
            *calls.entry(tool_name.to_string()).or_insert(0) += 1;
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}