use anyhow::Result;
use arkavo_authorization::{AuthorizationClient, AuthorizationError};
use arkavo_mcp::{RpcError, RpcRequest, RpcResponse, Tool, ToolRequest, ToolResponse, error_codes};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// MCP server that manages tools and handles requests
pub struct McpServer {
    tools: Arc<RwLock<HashMap<String, Arc<dyn Tool>>>>,
    pep: Option<Arc<AuthorizationClient>>,
}

impl McpServer {
    /// Create a new MCP server
    pub fn new() -> Self {
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
            pep: None,
        }
    }

    #[must_use]
    pub fn with_pep(mut self, pep: Arc<AuthorizationClient>) -> Self {
        self.pep = Some(pep);
        self
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
                    meta: None,
                },
                Err(e) => ToolResponse {
                    tool_name: request.tool_name,
                    result: serde_json::json!({
                        "error": e.to_string()
                    }),
                    success: false,
                    meta: None,
                },
            },
            None => ToolResponse {
                tool_name: request.tool_name.clone(),
                result: serde_json::json!({
                    "error": format!("Tool '{}' not found", request.tool_name)
                }),
                success: false,
                meta: None,
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

    async fn list_mcp_tools(&self) -> Value {
        let tools = self.tools.read().await;
        let listed: Vec<Value> = tools
            .values()
            .map(|tool| {
                let schema = tool.schema();
                serde_json::json!({
                    "name": schema.name,
                    "description": schema.description,
                    "inputSchema": schema.parameters
                })
            })
            .collect();
        serde_json::json!({ "tools": listed })
    }

    async fn pep_gate(
        &self,
        method: &str,
        params: Option<&Value>,
        subject_cwt: Option<&str>,
    ) -> Option<RpcError> {
        let Some(pep) = &self.pep else {
            return None;
        };
        match pep.authorize_mcp_method(method, params, subject_cwt).await {
            Ok(_) => None,
            Err(e) => Some(pep_rpc_error(&e)),
        }
    }

    /// Handle a JSON-RPC request. `subject_cwt` is the inbound Bearer or CWT-only env.
    pub async fn handle_rpc_request(&self, request: RpcRequest) -> RpcResponse {
        self.handle_rpc_request_with_subject(request, None).await
    }

    pub async fn handle_rpc_request_with_subject(
        &self,
        request: RpcRequest,
        subject_cwt: Option<&str>,
    ) -> RpcResponse {
        debug!("Handling RPC request: method={}", request.method);

        if request.method == "ping" || request.method.starts_with("notifications/") {
            return rpc_ok(request.id, serde_json::json!({}));
        }

        match request.method.as_str() {
            "tools/call" => {
                if let Some(err) = self
                    .pep_gate("tools/call", request.params.as_ref(), subject_cwt)
                    .await
                {
                    return rpc_err(request.id, err);
                }
                self.handle_tools_call(request).await
            }
            "tools/list" => {
                if let Some(err) = self
                    .pep_gate("tools/list", request.params.as_ref(), subject_cwt)
                    .await
                {
                    return rpc_err(request.id, err);
                }
                rpc_ok(request.id, self.list_mcp_tools().await)
            }
            "execute_tool" => match request.params {
                Some(params) => match serde_json::from_value::<ToolRequest>(params.clone()) {
                    Ok(tool_request) => {
                        let mapped = serde_json::json!({ "name": tool_request.tool_name });
                        if let Some(err) = self
                            .pep_gate("tools/call", Some(&mapped), subject_cwt)
                            .await
                        {
                            return rpc_err(request.id, err);
                        }
                        let response = self.execute_tool(tool_request).await;
                        rpc_ok(
                            request.id,
                            serde_json::to_value(response).unwrap_or(Value::Null),
                        )
                    }
                    Err(e) => rpc_err(
                        request.id,
                        RpcError {
                            code: error_codes::INVALID_PARAMS,
                            message: format!("Invalid tool request: {e}"),
                            data: None,
                        },
                    ),
                },
                None => rpc_err(
                    request.id,
                    RpcError {
                        code: error_codes::INVALID_PARAMS,
                        message: "Missing parameters".to_string(),
                        data: None,
                    },
                ),
            },
            "list_tools" => {
                if let Some(err) = self
                    .pep_gate("tools/list", request.params.as_ref(), subject_cwt)
                    .await
                {
                    return rpc_err(request.id, err);
                }
                let tools = self.list_tools().await;
                rpc_ok(
                    request.id,
                    serde_json::to_value(tools).unwrap_or(Value::Null),
                )
            }
            "get_tool_schema" => {
                let tool_name = request
                    .params
                    .as_ref()
                    .and_then(|p| p.get("tool_name"))
                    .and_then(|v| v.as_str());

                match tool_name {
                    Some(tool_name) => match self.get_tool_schema(tool_name).await {
                        Some(schema) => rpc_ok(request.id, schema),
                        None => rpc_err(
                            request.id,
                            RpcError {
                                code: error_codes::INVALID_PARAMS,
                                message: format!("Tool '{tool_name}' not found"),
                                data: None,
                            },
                        ),
                    },
                    None => rpc_err(
                        request.id,
                        RpcError {
                            code: error_codes::INVALID_PARAMS,
                            message: "Missing 'tool_name' parameter".to_string(),
                            data: None,
                        },
                    ),
                }
            }
            _ if self.pep.is_some() => rpc_err(
                request.id,
                RpcError {
                    code: error_codes::AUTHORIZATION_DENIED,
                    message: format!("Unknown method denied: {}", request.method),
                    data: None,
                },
            ),
            _ => rpc_err(
                request.id,
                RpcError {
                    code: error_codes::METHOD_NOT_FOUND,
                    message: format!("Unknown method: {}", request.method),
                    data: None,
                },
            ),
        }
    }

    async fn handle_tools_call(&self, request: RpcRequest) -> RpcResponse {
        let Some(params) = request.params else {
            return rpc_err(
                request.id,
                RpcError {
                    code: error_codes::INVALID_PARAMS,
                    message: "Missing parameters".to_string(),
                    data: None,
                },
            );
        };
        let Some(tool_name) = params
            .get("name")
            .or_else(|| params.get("tool_name"))
            .and_then(|v| v.as_str())
        else {
            return rpc_err(
                request.id,
                RpcError {
                    code: error_codes::INVALID_PARAMS,
                    message: "tools/call requires params.name".to_string(),
                    data: None,
                },
            );
        };
        let args = params
            .get("arguments")
            .cloned()
            .or_else(|| params.get("params").cloned())
            .unwrap_or(Value::Null);
        let response = self
            .execute_tool(ToolRequest {
                tool_name: tool_name.to_string(),
                params: args,
            })
            .await;
        if response.success {
            rpc_ok(
                request.id,
                serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": response.result.to_string()
                    }]
                }),
            )
        } else {
            rpc_err(
                request.id,
                RpcError {
                    code: error_codes::INTERNAL_ERROR,
                    message: response.result.to_string(),
                    data: None,
                },
            )
        }
    }
}

fn pep_rpc_error(err: &AuthorizationError) -> RpcError {
    RpcError {
        code: err.jsonrpc_code(),
        message: err.to_string(),
        data: None,
    }
}

fn rpc_ok(id: Option<Value>, result: Value) -> RpcResponse {
    RpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: Some(result),
        error: None,
    }
}

fn rpc_err(id: Option<Value>, error: RpcError) -> RpcResponse {
    RpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: None,
        error: Some(error),
    }
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}
