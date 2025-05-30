use arkavo_test::TestHarness;
use arkavo_test::mcp::server::ToolRequest;
use jsonschema::{Draft, JSONSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
use std::sync::OnceLock;

// Standard JSON-RPC error codes
const PARSE_ERROR: i32 = -32700;
const INVALID_REQUEST: i32 = -32600;
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;
const INTERNAL_ERROR: i32 = -32603;

// Schema validators
static REQUEST_SCHEMA: OnceLock<JSONSchema> = OnceLock::new();
static RESPONSE_SCHEMA: OnceLock<JSONSchema> = OnceLock::new();

fn init_schemas() {
    // JSON-RPC Request Schema based on MCP TypeScript definitions
    let request_schema = json!({
        "type": "object",
        "properties": {
            "jsonrpc": {
                "type": "string",
                "const": "2.0"
            },
            "id": {
                "oneOf": [
                    {"type": "string"},
                    {"type": "number"}
                ]
            },
            "method": {
                "type": "string"
            },
            "params": {
                "type": "object"
            }
        },
        "required": ["jsonrpc", "id", "method"],
        "additionalProperties": false
    });

    // JSON-RPC Response Schema (success or error)
    let response_schema = json!({
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "jsonrpc": {
                        "type": "string",
                        "const": "2.0"
                    },
                    "id": {
                        "oneOf": [
                            {"type": "string"},
                            {"type": "number"}
                        ]
                    },
                    "result": {}
                },
                "required": ["jsonrpc", "id", "result"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "jsonrpc": {
                        "type": "string",
                        "const": "2.0"
                    },
                    "id": {
                        "oneOf": [
                            {"type": "string"},
                            {"type": "number"}
                        ]
                    },
                    "error": {
                        "type": "object",
                        "properties": {
                            "code": {"type": "integer"},
                            "message": {"type": "string"},
                            "data": {}
                        },
                        "required": ["code", "message"],
                        "additionalProperties": false
                    }
                },
                "required": ["jsonrpc", "id", "error"],
                "additionalProperties": false
            }
        ]
    });

    REQUEST_SCHEMA.set(JSONSchema::options()
        .with_draft(Draft::Draft7)
        .compile(&request_schema)
        .expect("Failed to compile request schema"))
        .expect("Failed to set request schema");

    RESPONSE_SCHEMA.set(JSONSchema::options()
        .with_draft(Draft::Draft7)
        .compile(&response_schema)
        .expect("Failed to compile response schema"))
        .expect("Failed to set response schema");
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
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

fn validate_request(request: &Value) -> Result<(), String> {
    let schema = REQUEST_SCHEMA.get().expect("Schema not initialized");
    let result = schema.validate(request);
    
    if let Err(errors) = result {
        let error_messages: Vec<String> = errors
            .map(|e| format!("{}: {}", e.instance_path, e))
            .collect();
        return Err(format!("Request validation failed: {}", error_messages.join(", ")));
    }
    
    Ok(())
}

fn validate_response(response: &Value) -> Result<(), String> {
    let schema = RESPONSE_SCHEMA.get().expect("Schema not initialized");
    let result = schema.validate(response);
    
    if let Err(errors) = result {
        let error_messages: Vec<String> = errors
            .map(|e| format!("{}: {}", e.instance_path, e))
            .collect();
        return Err(format!("Response validation failed: {}", error_messages.join(", ")));
    }
    
    Ok(())
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
    // Initialize schemas
    init_schemas();
    
    // Initialize test harness
    let harness = TestHarness::new()
        .map_err(|e| anyhow::anyhow!("Failed to initialize test harness: {}", e))?;

    let mcp_server = harness.mcp_server();

    eprintln!("Arkavo MCP Server starting with schema validation...");
    
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

        // Parse as JSON first
        let json_value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("JSON parse error: {}", e);
                // For parse errors, we can't send a proper error response
                // because we don't have a request ID
                continue;
            }
        };

        // Validate request schema
        if let Err(e) = validate_request(&json_value) {
            eprintln!("Request validation error: {}", e);
            
            // Try to extract ID for error response
            if let Some(id) = json_value.get("id") {
                if let Ok(id_val) = serde_json::from_value::<JsonRpcId>(id.clone()) {
                    let error_resp = error_response(
                        id_val,
                        INVALID_REQUEST,
                        format!("Invalid request: {}", e),
                        None
                    );
                    
                    let resp_json = serde_json::to_value(&error_resp)?;
                    if let Err(e) = validate_response(&resp_json) {
                        eprintln!("ERROR: Generated invalid error response: {}", e);
                        continue;
                    }
                    
                    writeln!(stdout, "{}", serde_json::to_string(&error_resp)?)?;
                    stdout.flush()?;
                }
            }
            continue;
        }

        // Parse into typed request
        let request: JsonRpcRequest = match serde_json::from_value(json_value) {
            Ok(req) => req,
            Err(e) => {
                eprintln!("Failed to parse request: {}", e);
                continue;
            }
        };

        // Handle request
        let response = match request.method.as_str() {
            "initialize" => success_response(request.id, json!({
                "protocolVersion": "2024-11-05",
                "serverInfo": {
                    "name": "arkavo",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "tools": {
                        "available": get_tool_list()
                    }
                }
            })),

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
                                    
                                    error_response(
                                        request.id,
                                        INTERNAL_ERROR,
                                        format!("{}: {}", error_code, error_msg),
                                        Some(tool_response.result)
                                    )
                                } else {
                                    // Normal successful response
                                    success_response(
                                        request.id,
                                        json!({
                                            "content": [{
                                                "type": "text",
                                                "text": serde_json::to_string_pretty(&tool_response.result)
                                                    .unwrap_or_else(|_| "Error serializing result".to_string())
                                            }]
                                        })
                                    )
                                }
                            },
                            Err(e) => error_response(
                                request.id,
                                INTERNAL_ERROR,
                                format!("Tool execution error: {}", e),
                                None
                            ),
                        }
                    } else {
                        error_response(
                            request.id,
                            INVALID_PARAMS,
                            "Invalid parameters".to_string(),
                            None
                        )
                    }
                } else {
                    error_response(
                        request.id,
                        INVALID_PARAMS,
                        "Missing parameters".to_string(),
                        None
                    )
                }
            }

            "tools/list" => success_response(request.id, json!({
                "tools": get_tool_list()
            })),

            _ => error_response(
                request.id,
                METHOD_NOT_FOUND,
                format!("Method not found: {}", request.method),
                None
            ),
        };

        // Validate response before sending
        let response_json = serde_json::to_value(&response)?;
        if let Err(e) = validate_response(&response_json) {
            eprintln!("ERROR: Generated invalid response: {}", e);
            eprintln!("Response was: {}", serde_json::to_string_pretty(&response_json)?);
            
            // Send internal error instead
            let error_resp = error_response(
                match &response {
                    JsonRpcResponse::Success(s) => s.id.clone(),
                    JsonRpcResponse::Error(e) => e.id.clone(),
                },
                INTERNAL_ERROR,
                "Internal server error: Invalid response generated".to_string(),
                None
            );
            
            writeln!(stdout, "{}", serde_json::to_string(&error_resp)?)?;
            stdout.flush()?;
            continue;
        }

        // Send validated response
        let response_str = serde_json::to_string(&response)?;
        eprintln!("MCP Server sending response: {}", response_str);
        writeln!(stdout, "{}", response_str)?;
        stdout.flush()?;
    }

    Ok(())
}

fn get_tool_list() -> Vec<Value> {
    vec![
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
            "name": "ui_interaction",
            "description": "Interact with iOS UI elements. IMPORTANT FOR AI AGENTS: 1) Always use screen_capture first to see the UI state. 2) For text input: tap the text field first, then use type_text. 3) Use analyze_layout for AI vision analysis of UI elements. 4) Coordinate-based interactions only (no text/accessibility selectors).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["tap", "swipe", "type_text", "press_button", "analyze_layout"]},
                    "device_id": {"type": "string"},
                    "target": {
                        "type": "object",
                        "properties": {
                            "x": {"type": "number"},
                            "y": {"type": "number"},
                            "text": {"type": "string"},
                            "accessibility_id": {"type": "string"}
                        }
                    },
                    "value": {"type": "string"},
                    "swipe": {
                        "type": "object",
                        "properties": {
                            "x1": {"type": "number"},
                            "y1": {"type": "number"},
                            "x2": {"type": "number"},
                            "y2": {"type": "number"},
                            "duration": {"type": "number"}
                        }
                    }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "screen_capture",
            "description": "Capture iOS screen. AI AGENTS: Use this before any UI interaction to see current state. The screenshot will be saved to test_results/<name>.png. You can then read the image file to analyze UI elements and their positions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "device_id": {"type": "string"},
                    "analyze": {"type": "boolean"}
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "ui_query",
            "description": "Query UI elements (LIMITED). AI AGENTS: This tool has limited functionality without XCTest. Instead, use this workflow: 1) screen_capture to get screenshot, 2) Read the image file, 3) Use your vision capabilities to identify UI elements and coordinates, 4) Use tap/swipe/type_text with those coordinates.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query_type": {"type": "string", "enum": ["accessibility_tree", "visible_elements", "text_content"]},
                    "device_id": {"type": "string"},
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
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
            "name": "biometric_auth",
            "description": "Handle Face ID/Touch ID authentication prompts",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["enroll", "match", "fail", "cancel"]},
                    "device_id": {"type": "string"},
                    "biometric_type": {"type": "string", "enum": ["face_id", "touch_id"]}
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "system_dialog",
            "description": "Handle iOS system dialogs and alerts",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["accept", "dismiss", "allow", "deny"]},
                    "device_id": {"type": "string"},
                    "button_text": {"type": "string"}
                },
                "required": ["action"]
            }
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
            "name": "deep_link",
            "description": "Open deep links or URLs in iOS apps to navigate directly to specific screens",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": {"type": "string"},
                    "device_id": {"type": "string"},
                    "bundle_id": {"type": "string"}
                },
                "required": ["url"]
            }
        }),
        json!({
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
        }),
        json!({
            "name": "list_tests",
            "description": "List all available tests in the repository",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filter": {"type": "string"},
                    "test_type": {"type": "string", "enum": ["unit", "integration", "performance", "ui", "all"]}
                }
            }
        })
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_validation() {
        init_schemas();
        
        // Valid request
        let valid_request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "test",
            "params": {}
        });
        assert!(validate_request(&valid_request).is_ok());
        
        // Missing jsonrpc
        let invalid_request = json!({
            "id": 1,
            "method": "test"
        });
        assert!(validate_request(&invalid_request).is_err());
        
        // Wrong jsonrpc version
        let invalid_request = json!({
            "jsonrpc": "1.0",
            "id": 1,
            "method": "test"
        });
        assert!(validate_request(&invalid_request).is_err());
        
        // Null id
        let invalid_request = json!({
            "jsonrpc": "2.0",
            "id": null,
            "method": "test"
        });
        assert!(validate_request(&invalid_request).is_err());
    }

    #[test]
    fn test_response_validation() {
        init_schemas();
        
        // Valid success response
        let valid_response = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {"test": "value"}
        });
        assert!(validate_response(&valid_response).is_ok());
        
        // Valid error response
        let valid_error = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32600,
                "message": "Invalid request"
            }
        });
        assert!(validate_response(&valid_error).is_ok());
        
        // Invalid - has both result and error
        let invalid_response = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {},
            "error": {
                "code": -32600,
                "message": "Invalid"
            }
        });
        assert!(validate_response(&invalid_response).is_err());
        
        // Invalid - missing required fields
        let invalid_response = json!({
            "jsonrpc": "2.0",
            "id": 1
        });
        assert!(validate_response(&invalid_response).is_err());
    }
}