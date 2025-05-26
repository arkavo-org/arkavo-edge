use arkavo_test::TestHarness;
use arkavo_test::mcp::server::ToolRequest;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Value,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize test harness
    let harness = TestHarness::new()
        .map_err(|e| anyhow::anyhow!("Failed to initialize test harness: {}", e))?;
    
    let mcp_server = harness.mcp_server();
    
    eprintln!("Arkavo MCP Server starting...");
    
    // Main request/response loop
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    
    let reader = io::BufReader::new(stdin);
    for line in reader.lines() {
        let line = line?;
        
        // Parse JSON-RPC request
        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(e) => {
                let error_response = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: json!(null),
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32700,
                        message: format!("Parse error: {}", e),
                        data: None,
                    }),
                };
                writeln!(stdout, "{}", serde_json::to_string(&error_response)?)?;
                stdout.flush()?;
                continue;
            }
        };
        
        // Handle request
        let response = match request.method.as_str() {
            "initialize" => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: Some(json!({
            "protocolVersion": "2024-11-05",
            "serverInfo": {
                "name": "arkavo",
                "version": "0.2.0"
            },
            "capabilities": {
                "tools": {
                    "available": [
                        {
                            "name": "query_state",
                            "description": "Query application state",
                            "parameters": {
                                "type": "object",
                                "properties": {
                                    "entity": {"type": "string"},
                                    "filter": {"type": "object"}
                                },
                                "required": ["entity"]
                            }
                        },
                        {
                            "name": "mutate_state",
                            "description": "Mutate application state",
                            "parameters": {
                                "type": "object",
                                "properties": {
                                    "entity": {"type": "string"},
                                    "action": {"type": "string"},
                                    "data": {"type": "object"}
                                },
                                "required": ["entity", "action"]
                            }
                        },
                        {
                            "name": "snapshot",
                            "description": "Manage state snapshots",
                            "parameters": {
                                "type": "object",
                                "properties": {
                                    "action": {"type": "string", "enum": ["create", "restore", "list"]},
                                    "name": {"type": "string"}
                                },
                                "required": ["action"]
                            }
                        },
                        {
                            "name": "run_test",
                            "description": "Execute test scenarios",
                            "parameters": {
                                "type": "object",
                                "properties": {
                                    "test_name": {"type": "string"},
                                    "timeout": {"type": "integer"}
                                },
                                "required": ["test_name"]
                            }
                        },
                        {
                            "name": "ui_interaction",
                            "description": "Interact with iOS UI elements",
                            "parameters": {
                                "type": "object",
                                "properties": {
                                    "action": {"type": "string", "enum": ["tap", "swipe", "type_text", "press_button"]},
                                    "target": {
                                        "type": "object",
                                        "properties": {
                                            "x": {"type": "number"},
                                            "y": {"type": "number"},
                                            "text": {"type": "string"},
                                            "accessibility_id": {"type": "string"}
                                        }
                                    },
                                    "value": {"type": "string"}
                                },
                                "required": ["action"]
                            }
                        },
                        {
                            "name": "screen_capture",
                            "description": "Capture and analyze iOS screen",
                            "parameters": {
                                "type": "object",
                                "properties": {
                                    "name": {"type": "string"},
                                    "analyze": {"type": "boolean"}
                                },
                                "required": ["name"]
                            }
                        },
                        {
                            "name": "ui_query",
                            "description": "Query UI element state and properties",
                            "parameters": {
                                "type": "object",
                                "properties": {
                                    "query_type": {"type": "string", "enum": ["accessibility_tree", "visible_elements", "text_content"]},
                                    "filter": {
                                        "type": "object",
                                        "properties": {
                                            "element_type": {"type": "string"},
                                            "text_contains": {"type": "string"},
                                            "accessibility_label": {"type": "string"}
                                        }
                                    }
                                },
                                "required": ["query_type"]
                            }
                        }
                    ]
                }
            }
        })),
        error: None,
    },
            
            "tools/call" => {
                if let Some(params) = request.params {
                    if let (Some(tool_name), Some(args)) = (
                        params.get("name").and_then(|v| v.as_str()),
                        params.get("arguments")
                    ) {
                        let tool_request = ToolRequest {
                            tool_name: tool_name.to_string(),
                            params: args.clone(),
                        };
                        
                        match mcp_server.call_tool(tool_request).await {
                            Ok(tool_response) => JsonRpcResponse {
                                jsonrpc: "2.0".to_string(),
                                id: request.id,
                                result: Some(json!({
                                    "tool_name": tool_response.tool_name,
                                    "result": tool_response.result,
                                    "success": tool_response.success
                                })),
                                error: None,
                            },
                            Err(e) => JsonRpcResponse {
                                jsonrpc: "2.0".to_string(),
                                id: request.id,
                                result: None,
                                error: Some(JsonRpcError {
                                    code: -32603,
                                    message: format!("Tool execution error: {}", e),
                                    data: None,
                                }),
                            }
                        }
                    } else {
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: request.id,
                            result: None,
                            error: Some(JsonRpcError {
                                code: -32602,
                                message: "Invalid parameters".to_string(),
                                data: None,
                            }),
                        }
                    }
                } else {
                    JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: request.id,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32602,
                            message: "Missing parameters".to_string(),
                            data: None,
                        }),
                    }
                }
            }
            
            "tools/list" => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: Some(json!({
                    "tools": [
                        {"name": "query_state", "description": "Query application state"},
                        {"name": "mutate_state", "description": "Mutate application state"},
                        {"name": "snapshot", "description": "Manage state snapshots"},
                        {"name": "run_test", "description": "Execute test scenarios"},
                        {"name": "ui_interaction", "description": "Interact with iOS UI elements"},
                        {"name": "screen_capture", "description": "Capture and analyze iOS screen"},
                        {"name": "ui_query", "description": "Query UI element state and properties"}
                    ]
                })),
                error: None,
            },
            
            _ => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32601,
                    message: format!("Method not found: {}", request.method),
                    data: None,
                }),
            }
        };
        
        // Send response
        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }
    
    Ok(())
}