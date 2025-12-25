use anyhow::Result;
use arkavo_mcp::{RpcError, RpcRequest, RpcResponse, Tool, ToolRequest, ToolResponse, error_codes};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// MCP server that manages tools and handles requests
pub struct McpServer {
    tools: Arc<RwLock<HashMap<String, Arc<dyn Tool>>>>,
}

impl McpServer {
    /// Create a new MCP server
    pub fn new() -> Self {
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a tool with the server
    pub async fn register_tool(&self, name: String, tool: Arc<dyn Tool>) -> Result<()> {
        let mut tools = self.tools.write().await;
        if tools.contains_key(&name) {
            return Err(anyhow::anyhow!("Tool '{name}' is already registered"));
        }
        info!("Registered tool: {}", name);
        tools.insert(name, tool);
        drop(tools);
        Ok(())
    }

    /// Register a tool, replacing if it already exists
    pub async fn register_or_replace_tool(&self, name: String, tool: Arc<dyn Tool>) {
        let mut tools = self.tools.write().await;
        info!("Registered tool: {} (replacing if exists)", name);
        tools.insert(name, tool);
    }

    /// Unregister a tool from the server
    pub async fn unregister_tool(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let mut tools = self.tools.write().await;
        let removed = tools.remove(name);
        if removed.is_some() {
            info!("Unregistered tool: {}", name);
        }
        removed
    }

    /// Execute a tool by name
    pub async fn execute_tool(&self, request: ToolRequest) -> ToolResponse {
        let tools = self.tools.read().await;

        match tools.get(&request.tool_name) {
            Some(tool) => match tool.execute(request.params).await {
                Ok(result) => ToolResponse {
                    tool_name: request.tool_name,
                    result,
                    success: true,
                },
                Err(e) => ToolResponse {
                    tool_name: request.tool_name,
                    result: serde_json::json!({
                        "error": e.to_string()
                    }),
                    success: false,
                },
            },
            None => ToolResponse {
                tool_name: request.tool_name.clone(),
                result: serde_json::json!({
                    "error": format!("Tool '{}' not found", request.tool_name)
                }),
                success: false,
            },
        }
    }

    /// List all available tools
    pub async fn list_tools(&self) -> Vec<String> {
        let tools = self.tools.read().await;
        tools.keys().cloned().collect()
    }

    /// Get schema for a specific tool
    pub async fn get_tool_schema(&self, tool_name: &str) -> Option<Value> {
        let tools = self.tools.read().await;
        tools.get(tool_name).map(|tool| {
            let schema = tool.schema();
            serde_json::to_value(schema).unwrap_or(Value::Null)
        })
    }

    /// Handle a JSON-RPC request
    pub async fn handle_rpc_request(&self, request: RpcRequest) -> RpcResponse {
        debug!("Handling RPC request: method={}", request.method);

        match request.method.as_str() {
            "execute_tool" => match request.params {
                Some(params) => match serde_json::from_value::<ToolRequest>(params) {
                    Ok(tool_request) => {
                        let response = self.execute_tool(tool_request).await;
                        RpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: request.id,
                            result: Some(serde_json::to_value(response).unwrap_or(Value::Null)),
                            error: None,
                        }
                    }
                    Err(e) => RpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: request.id,
                        result: None,
                        error: Some(RpcError {
                            code: error_codes::INVALID_PARAMS,
                            message: format!("Invalid tool request: {e}"),
                            data: None,
                        }),
                    },
                },
                None => RpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: None,
                    error: Some(RpcError {
                        code: error_codes::INVALID_PARAMS,
                        message: "Missing parameters".to_string(),
                        data: None,
                    }),
                },
            },
            "list_tools" => {
                let tools = self.list_tools().await;
                RpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: Some(serde_json::to_value(tools).unwrap_or(Value::Null)),
                    error: None,
                }
            }
            "get_tool_schema" => {
                let tool_name = request
                    .params
                    .as_ref()
                    .and_then(|p| p.get("tool_name"))
                    .and_then(|v| v.as_str());

                match tool_name {
                    Some(tool_name) => match self.get_tool_schema(tool_name).await {
                        Some(schema) => RpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: request.id,
                            result: Some(schema),
                            error: None,
                        },
                        None => RpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: request.id,
                            result: None,
                            error: Some(RpcError {
                                code: error_codes::INVALID_PARAMS,
                                message: format!("Tool '{tool_name}' not found"),
                                data: None,
                            }),
                        },
                    },
                    None => RpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: request.id,
                        result: None,
                        error: Some(RpcError {
                            code: error_codes::INVALID_PARAMS,
                            message: "Missing 'tool_name' parameter".to_string(),
                            data: None,
                        }),
                    },
                }
            }
            _ => RpcResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: None,
                error: Some(RpcError {
                    code: error_codes::METHOD_NOT_FOUND,
                    message: format!("Unknown method: {}", request.method),
                    data: None,
                }),
            },
        }
    }
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}
