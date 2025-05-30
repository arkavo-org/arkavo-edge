use arkavo_test::TestHarness;
use arkavo_test::mcp::server::ToolRequest;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

// Standard JSON-RPC error codes
const PARSE_ERROR: i32 = -32700;
const INVALID_REQUEST: i32 = -32600;
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;
const INTERNAL_ERROR: i32 = -32603;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    #[serde(deserialize_with = "deserialize_id")]
    id: JsonRpcId,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum JsonRpcId {
    Number(i64),
    String(String),
}

fn deserialize_id<'de, D>(deserializer: D) -> Result<JsonRpcId, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(JsonRpcId::Number(i))
            } else {
                Err(serde::de::Error::custom("id must be an integer or string"))
            }
        }
        Value::String(s) => Ok(JsonRpcId::String(s)),
        _ => Err(serde::de::Error::custom("id must be a number or string")),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum JsonRpcResponse {
    Success(JsonRpcSuccessResponse),
    Error(JsonRpcErrorResponse),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcSuccessResponse {
    jsonrpc: String,
    id: JsonRpcId,
    result: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcErrorResponse {
    jsonrpc: String,
    id: JsonRpcId,
    error: JsonRpcError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcNotification {
    jsonrpc: String,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

fn success_response(id: JsonRpcId, result: Value) -> JsonRpcResponse {
    JsonRpcResponse::Success(JsonRpcSuccessResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result,
    })
}

fn error_response(id: JsonRpcId, code: i32, message: String, data: Option<Value>) -> JsonRpcResponse {
    JsonRpcResponse::Error(JsonRpcErrorResponse {
        jsonrpc: "2.0".to_string(),
        id,
        error: JsonRpcError { code, message, data },
    })
}

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize test harness
    let harness = TestHarness::new()
        .map_err(|e| anyhow::anyhow!("Failed to initialize test harness: {}", e))?;

    let mcp_server = harness.mcp_server();

    eprintln!("Arkavo MCP Server starting...");
    
    // Set up panic handler to ensure clean JSON-RPC error on panic
    std::panic::set_hook(Box::new(|panic_info| {
        eprintln!("MCP Server panic: {:?}", panic_info);
        // Don't output to stdout to avoid corrupting JSON-RPC stream
    }));

    // Main request/response loop
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    let reader = io::BufReader::new(stdin);
    for line in reader.lines() {
        let line = match line {
            Ok(l) => {
                eprintln!("MCP Server received: {}", l);
                l
            },
            Err(e) => {
                eprintln!("Error reading input: {}", e);
                continue;
            }
        };

        // Parse JSON-RPC request
        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(e) => {
                // For parse errors, we can't get the ID from the request
                // The JSON-RPC spec says to omit the response entirely for parse errors
                // or send a notification (no id field) if we must respond
                eprintln!("Parse error: {}", e);
                // Skip this malformed request
                continue;
            }
        };

        // Handle request
        let response = match request.method.as_str() {
            "initialize" => JsonRpcResponse::Success(JsonRpcSuccessResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: json!({
                    "protocolVersion": "2024-11-05",
                    "serverInfo": {
                        "name": "arkavo",
                        "version": env!("CARGO_PKG_VERSION")
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
                                },
                                {
                                    "name": "passkey_dialog",
                                    "description": "Handle iOS passkey/biometric enrollment dialogs",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "action": {"type": "string", "enum": ["dismiss_enrollment_warning", "accept_enrollment", "cancel_dialog", "tap_settings"]},
                                            "device_id": {"type": "string"}
                                        },
                                        "required": ["action"]
                                    }
                                },
                                {
                                    "name": "simulator_control",
                                    "description": "Control iOS simulators - boot, shutdown, list devices",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "action": {"type": "string", "enum": ["list", "boot", "shutdown", "refresh"]},
                                            "device_id": {"type": "string"}
                                        },
                                        "required": ["action"]
                                    }
                                },
                                {
                                    "name": "app_management",
                                    "description": "Manage iOS apps - install, uninstall, launch, terminate",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "action": {"type": "string", "enum": ["install", "uninstall", "launch", "terminate", "list"]},
                                            "device_id": {"type": "string"},
                                            "app_path": {"type": "string"},
                                            "bundle_id": {"type": "string"},
                                            "arguments": {"type": "array", "items": {"type": "string"}}
                                        },
                                        "required": ["action", "device_id"]
                                    }
                                },
                                {
                                    "name": "file_operations",
                                    "description": "Transfer files to/from iOS simulator",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "action": {"type": "string", "enum": ["push", "pull", "get_container"]},
                                            "device_id": {"type": "string"},
                                            "local_path": {"type": "string"},
                                            "remote_path": {"type": "string"},
                                            "bundle_id": {"type": "string"}
                                        },
                                        "required": ["action", "device_id"]
                                    }
                                },
                                {
                                    "name": "device_management",
                                    "description": "Manage iOS devices and simulators",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "action": {"type": "string", "enum": ["list", "set_active", "get_active", "refresh"]},
                                            "device_id": {"type": "string"}
                                        },
                                        "required": ["action"]
                                    }
                                },
                                {
                                    "name": "coordinate_converter",
                                    "description": "Convert between screen and element coordinates",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "action": {"type": "string", "enum": ["convert", "validate"]},
                                            "x": {"type": "number"},
                                            "y": {"type": "number"},
                                            "device_type": {"type": "string"},
                                            "coordinate_type": {"type": "string", "enum": ["screen", "element", "normalized"]}
                                        },
                                        "required": ["action", "x", "y"]
                                    }
                                },
                                {
                                    "name": "deep_link",
                                    "description": "Handle deep links and URL schemes",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "url": {"type": "string"},
                                            "device_id": {"type": "string"}
                                        },
                                        "required": ["url"]
                                    }
                                },
                                {
                                    "name": "app_launcher",
                                    "description": "Launch, terminate, or get info about iOS apps",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "action": {"type": "string", "enum": ["launch", "terminate", "install", "uninstall", "list", "info"]},
                                            "bundle_id": {"type": "string"},
                                            "device_id": {"type": "string"},
                                            "app_path": {"type": "string"},
                                            "launch_args": {"type": "array", "items": {"type": "string"}},
                                            "env": {"type": "object"}
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
                                            "filter": {"type": "string"},
                                            "test_type": {"type": "string", "enum": ["unit", "integration", "performance", "ui", "all"]}
                                        }
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
                        params.get("arguments"),
                    ) {
                        let tool_request = ToolRequest {
                            tool_name: tool_name.to_string(),
                            params: args.clone(),
                        };

                        match mcp_server.call_tool(tool_request).await {
                            Ok(tool_response) => {
                                // Check if the tool returned an error object
                                if let Some(error_obj) = tool_response.result.get("error") {
                                    // Tool returned an error - convert to JSON-RPC error
                                    let error_code = error_obj.get("code")
                                        .and_then(|c| c.as_str())
                                        .unwrap_or("TOOL_ERROR");
                                    let error_msg = error_obj.get("message")
                                        .and_then(|m| m.as_str())
                                        .unwrap_or("Tool execution failed");
                                    
                                    JsonRpcResponse {
                                        jsonrpc: "2.0".to_string(),
                                        id: request.id,
                                        result: None,
                                        error: Some(JsonRpcError {
                                            code: INTERNAL_ERROR,
                                            message: format!("{}: {}", error_code, error_msg),
                                            data: Some(tool_response.result),
                                        }),
                                    }
                                } else {
                                    // Normal successful response
                                    JsonRpcResponse {
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
                                    }
                                }
                            },
                            Err(e) => JsonRpcResponse {
                                jsonrpc: "2.0".to_string(),
                                id: request.id,
                                result: None,
                                error: Some(JsonRpcError {
                                    code: INTERNAL_ERROR,
                                    message: format!("Tool execution error: {}", e),
                                    data: None,
                                }),
                            },
                        }
                    } else {
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: request.id,
                            result: None,
                            error: Some(JsonRpcError {
                                code: INVALID_PARAMS,
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
                            code: INVALID_PARAMS,
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
                            "name": "passkey_dialog",
                            "description": "Handle iOS passkey/biometric enrollment dialogs",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "action": {"type": "string", "enum": ["dismiss_enrollment_warning", "accept_enrollment", "cancel_dialog", "tap_settings"]},
                                    "device_id": {"type": "string"}
                                },
                                "required": ["action"]
                            }
                        },
                        {
                            "name": "simulator_control",
                            "description": "Control iOS simulators - boot, shutdown, list devices",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "action": {"type": "string", "enum": ["list", "boot", "shutdown", "refresh"]},
                                    "device_id": {"type": "string"}
                                },
                                "required": ["action"]
                            }
                        },
                        {
                            "name": "app_management",
                            "description": "Manage iOS apps - install, uninstall, launch, terminate",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "action": {"type": "string", "enum": ["install", "uninstall", "launch", "terminate", "list"]},
                                    "device_id": {"type": "string"},
                                    "app_path": {"type": "string"},
                                    "bundle_id": {"type": "string"},
                                    "arguments": {"type": "array", "items": {"type": "string"}}
                                },
                                "required": ["action", "device_id"]
                            }
                        },
                        {
                            "name": "file_operations",
                            "description": "Transfer files to/from iOS simulator",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "action": {"type": "string", "enum": ["push", "pull", "get_container"]},
                                    "device_id": {"type": "string"},
                                    "local_path": {"type": "string"},
                                    "remote_path": {"type": "string"},
                                    "bundle_id": {"type": "string"}
                                },
                                "required": ["action", "device_id"]
                            }
                        },
                        {
                            "name": "device_management",
                            "description": "Manage iOS devices and simulators",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "action": {"type": "string", "enum": ["list", "set_active", "get_active", "refresh"]},
                                    "device_id": {"type": "string"}
                                },
                                "required": ["action"]
                            }
                        },
                        {
                            "name": "coordinate_converter",
                            "description": "Convert between screen and element coordinates",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "action": {"type": "string", "enum": ["convert", "validate"]},
                                    "x": {"type": "number"},
                                    "y": {"type": "number"},
                                    "device_type": {"type": "string"},
                                    "coordinate_type": {"type": "string", "enum": ["screen", "element", "normalized"]}
                                },
                                "required": ["action", "x", "y"]
                            }
                        },
                        {
                            "name": "deep_link",
                            "description": "Handle deep links and URL schemes",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "url": {"type": "string"},
                                    "device_id": {"type": "string"}
                                },
                                "required": ["url"]
                            }
                        },
                        {
                            "name": "app_launcher",
                            "description": "Launch, terminate, or get info about iOS apps",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "action": {"type": "string", "enum": ["launch", "terminate", "install", "uninstall", "list", "info"]},
                                    "bundle_id": {"type": "string"},
                                    "device_id": {"type": "string"},
                                    "app_path": {"type": "string"},
                                    "launch_args": {"type": "array", "items": {"type": "string"}},
                                    "env": {"type": "object"}
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
                    code: METHOD_NOT_FOUND,
                    message: format!("Method not found: {}", request.method),
                    data: None,
                }),
            },
        };

        // Send response
        let response_str = serde_json::to_string(&response)?;
        eprintln!("MCP Server sending response: {}", response_str);
        writeln!(stdout, "{}", response_str)?;
        stdout.flush()?;
    }

    Ok(())
}
