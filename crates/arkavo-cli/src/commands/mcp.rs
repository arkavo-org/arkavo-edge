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
                            "inputSchema": {
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
                            "inputSchema": {
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
                            "inputSchema": {
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
                            "inputSchema": {
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
                            "inputSchema": {
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
                            "inputSchema": {
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
                            "inputSchema": {
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
                        },
                        {
                            "name": "find_bugs",
                            "description": "Find potential bugs and code issues in the codebase",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": {"type": "string"},
                                    "language": {"type": "string", "enum": ["rust", "swift", "typescript", "python", "auto"]},
                                    "bug_types": {"type": "array", "items": {"type": "string"}}
                                }
                            }
                        },
                        {
                            "name": "intelligent_bug_finder",
                            "description": "Use AI to find complex bugs in specific code modules",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "module": {"type": "string"},
                                    "context": {"type": "string"},
                                    "focus_areas": {"type": "array", "items": {"type": "string"}}
                                },
                                "required": ["module"]
                            }
                        },
                        {
                            "name": "discover_invariants",
                            "description": "Discover invariants that should always be true in a system",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "system": {"type": "string"},
                                    "code_context": {"type": "string"}
                                },
                                "required": ["system"]
                            }
                        },
                        {
                            "name": "chaos_test",
                            "description": "Test system behavior under failure conditions",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "scenario": {"type": "string"},
                                    "system_state": {"type": "object"},
                                    "failure_types": {"type": "array", "items": {"type": "string"}}
                                },
                                "required": ["scenario"]
                            }
                        },
                        {
                            "name": "explore_edge_cases",
                            "description": "Explore edge cases in system flows",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "flow": {"type": "string"},
                                    "known_cases": {"type": "array", "items": {"type": "string"}},
                                    "depth": {"type": "string", "enum": ["shallow", "deep", "exhaustive"]}
                                },
                                "required": ["flow"]
                            }
                        },
                        {
                            "name": "biometric_auth",
                            "description": "Handle Face ID/Touch ID authentication prompts",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "action": {"type": "string", "enum": ["enroll", "match", "fail", "cancel"]},
                                    "biometric_type": {"type": "string", "enum": ["face_id", "touch_id"]}
                                },
                                "required": ["action"]
                            }
                        },
                        {
                            "name": "system_dialog",
                            "description": "Handle iOS system dialogs and alerts",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "action": {"type": "string", "enum": ["accept", "dismiss", "allow", "deny"]},
                                    "button_text": {"type": "string"}
                                },
                                "required": ["action"]
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
                                    "content": [{
                                        "type": "text",
                                        "text": serde_json::to_string_pretty(&tool_response.result)
                                            .unwrap_or_else(|_| "Error serializing result".to_string())
                                    }]
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
                        {
                            "name": "query_state", 
                            "description": "Query application state",
                            "inputSchema": {
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
                            "inputSchema": {
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
                            "inputSchema": {
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
                            "inputSchema": {
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
                            "inputSchema": {
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
                            "inputSchema": {
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
                            "inputSchema": {
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
                        },
                        {
                            "name": "find_bugs", 
                            "description": "Find potential bugs and code issues in the codebase",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": {"type": "string"},
                                    "language": {"type": "string", "enum": ["rust", "swift", "typescript", "python", "auto"]},
                                    "bug_types": {"type": "array", "items": {"type": "string"}}
                                }
                            }
                        },
                        {
                            "name": "intelligent_bug_finder", 
                            "description": "Use AI to find complex bugs in specific code modules",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "module": {"type": "string"},
                                    "context": {"type": "string"},
                                    "focus_areas": {"type": "array", "items": {"type": "string"}}
                                },
                                "required": ["module"]
                            }
                        },
                        {
                            "name": "discover_invariants", 
                            "description": "Discover invariants that should always be true in a system",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "system": {"type": "string"},
                                    "code_context": {"type": "string"}
                                },
                                "required": ["system"]
                            }
                        },
                        {
                            "name": "chaos_test", 
                            "description": "Test system behavior under failure conditions",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "scenario": {"type": "string"},
                                    "system_state": {"type": "object"},
                                    "failure_types": {"type": "array", "items": {"type": "string"}}
                                },
                                "required": ["scenario"]
                            }
                        },
                        {
                            "name": "explore_edge_cases", 
                            "description": "Explore edge cases in system flows",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "flow": {"type": "string"},
                                    "known_cases": {"type": "array", "items": {"type": "string"}},
                                    "depth": {"type": "string", "enum": ["shallow", "deep", "exhaustive"]}
                                },
                                "required": ["flow"]
                            }
                        },
                        {
                            "name": "biometric_auth", 
                            "description": "Handle Face ID/Touch ID authentication prompts",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "action": {"type": "string", "enum": ["enroll", "match", "fail", "cancel"]},
                                    "biometric_type": {"type": "string", "enum": ["face_id", "touch_id"]}
                                },
                                "required": ["action"]
                            }
                        },
                        {
                            "name": "system_dialog", 
                            "description": "Handle iOS system dialogs and alerts",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "action": {"type": "string", "enum": ["accept", "dismiss", "allow", "deny"]},
                                    "button_text": {"type": "string"}
                                },
                                "required": ["action"]
                            }
                        },
                        {
                            "name": "list_tests",
                            "description": "List all available tests in the repository",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "filter": {
                                        "type": "string",
                                        "description": "Optional filter pattern for test names"
                                    },
                                    "test_type": {
                                        "type": "string",
                                        "enum": ["unit", "integration", "performance", "ui", "all"],
                                        "description": "Type of tests to list"
                                    }
                                },
                                "required": []
                            }
                        }
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