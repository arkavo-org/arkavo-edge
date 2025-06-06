use arkavo_test::TestHarness;
use arkavo_test::mcp::server::ToolRequest;
use jsonschema::{Draft, ValidationOptions, Validator};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
use std::sync::OnceLock;

// Standard JSON-RPC error codes
#[allow(dead_code)]
const PARSE_ERROR: i32 = -32700;
const INVALID_REQUEST: i32 = -32600;
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;
const INTERNAL_ERROR: i32 = -32603;

// Schema validators
static REQUEST_SCHEMA: OnceLock<Validator> = OnceLock::new();
static RESPONSE_SCHEMA: OnceLock<Validator> = OnceLock::new();

fn init_schemas() {
    // JSON-RPC Request Schema based on MCP TypeScript definitions
    // Supports both requests (with id) and notifications (without id)
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
        "required": ["jsonrpc", "method"],
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

    REQUEST_SCHEMA
        .set(
            ValidationOptions::default()
                .with_draft(Draft::Draft7)
                .build(&request_schema)
                .expect("Failed to compile request schema"),
        )
        .expect("Failed to set request schema");

    RESPONSE_SCHEMA
        .set(
            ValidationOptions::default()
                .with_draft(Draft::Draft7)
                .build(&response_schema)
                .expect("Failed to compile response schema"),
        )
        .expect("Failed to set response schema");
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<JsonRpcId>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcNotification {
    jsonrpc: String,
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
    let validator = REQUEST_SCHEMA.get().expect("Schema not initialized");

    if validator.validate(request).is_err() {
        let error_messages: Vec<String> = validator
            .iter_errors(request)
            .map(|e| format!("{}: {}", e.instance_path, e))
            .collect();
        return Err(format!(
            "Request validation failed: {}",
            error_messages.join(", ")
        ));
    }

    Ok(())
}

fn validate_response(response: &Value) -> Result<(), String> {
    let validator = RESPONSE_SCHEMA.get().expect("Schema not initialized");

    if validator.validate(response).is_err() {
        let error_messages: Vec<String> = validator
            .iter_errors(response)
            .map(|e| format!("{}: {}", e.instance_path, e))
            .collect();
        return Err(format!(
            "Response validation failed: {}",
            error_messages.join(", ")
        ));
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

fn error_response(
    id: JsonRpcId,
    code: i32,
    message: String,
    data: Option<Value>,
) -> JsonRpcResponse {
    JsonRpcResponse::Error(JsonRpcErrorResponse {
        jsonrpc: "2.0".to_string(),
        id,
        error: JsonRpcError {
            code,
            message,
            data,
        },
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
            }
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
                        None,
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

        // Check if this is a notification (no id field)
        if request.id.is_none() {
            eprintln!("Handling notification: {}", request.method);
            match request.method.as_str() {
                "notifications/initialized" => {
                    eprintln!("Client initialized notification received");
                }
                _ => {
                    eprintln!("Unknown notification: {}", request.method);
                }
            }
            continue; // Notifications don't get responses
        }

        // Handle request (has id, expects response)
        let request_id = request.id.clone().unwrap(); // Safe because we checked above
        let response = match request.method.as_str() {
            "initialize" => success_response(
                request_id.clone(),
                json!({
                    "protocolVersion": "2024-11-05",
                    "serverInfo": {
                        "name": "arkavo",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": {
                        "tools": {}
                    },
                    "instructions": "This MCP server provides iOS automation with XCUITest support. KEY IMPROVEMENTS:\n\n1. TEXT-BASED ELEMENT FINDING: Use {\"action\":\"tap\",\"target\":{\"text\":\"Button Label\"}} to tap elements by visible text. Much more reliable than coordinates.\n\n2. ACCESSIBILITY ID SUPPORT: Use {\"action\":\"tap\",\"target\":{\"accessibility_id\":\"element_id\"}} for the most reliable automation.\n\n3. BEST PRACTICES:\n   - Always start with screen_capture to see current UI\n   - Look for visible text in screenshots\n   - Use text/accessibility_id for interactions when possible\n   - Only use coordinates as last resort\n\n4. TEXT INPUT WORKFLOW:\n   - Tap the field first (by text label if possible)\n   - Use clear_text if needed\n   - Then use type_text with your value\n\n5. DEBUGGING: Error messages now provide helpful details. XCUITest waits up to 10 seconds for elements.\n\nFor detailed guidance, use the 'usage_guide' tool with topics: overview, text_based_tapping, workflows, debugging, or examples."
                }),
            ),

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
                                // Check if the tool returned an error object (and it's not null)
                                if let Some(error_obj) = tool_response.result.get("error") {
                                    if !error_obj.is_null() {
                                        // Tool returned an actual error - convert to JSON-RPC error
                                        let error_code = error_obj
                                            .get("code")
                                            .and_then(|c| c.as_str())
                                            .unwrap_or("TOOL_ERROR");
                                        let error_msg = error_obj
                                            .get("message")
                                            .and_then(|m| m.as_str())
                                            .unwrap_or("Tool execution failed");

                                        error_response(
                                            request_id.clone(),
                                            INTERNAL_ERROR,
                                            format!("{}: {}", error_code, error_msg),
                                            Some(tool_response.result),
                                        )
                                    } else {
                                        // error field is null, treat as success
                                        // Check response size before formatting
                                        let result_str =
                                            serde_json::to_string_pretty(&tool_response.result)
                                                .unwrap_or_else(|_| {
                                                    "Error serializing result".to_string()
                                                });

                                        // Limit response size to prevent MCP client errors
                                        let trimmed_result = if result_str.len() > 50_000 {
                                            eprintln!(
                                                "WARNING: Tool response too large ({} bytes), truncating",
                                                result_str.len()
                                            );
                                            format!(
                                                "{}\n\n... (truncated, original size: {} bytes)",
                                                &result_str[..50_000],
                                                result_str.len()
                                            )
                                        } else {
                                            result_str
                                        };

                                        // Normal successful response
                                        success_response(
                                            request_id.clone(),
                                            json!({
                                                "content": [{
                                                    "type": "text",
                                                    "text": trimmed_result
                                                }]
                                            }),
                                        )
                                    }
                                } else {
                                    // Check response size before formatting
                                    let result_str =
                                        serde_json::to_string_pretty(&tool_response.result)
                                            .unwrap_or_else(|_| {
                                                "Error serializing result".to_string()
                                            });

                                    // Limit response size to prevent MCP client errors
                                    let trimmed_result = if result_str.len() > 50_000 {
                                        eprintln!(
                                            "WARNING: Tool response too large ({} bytes), truncating",
                                            result_str.len()
                                        );
                                        format!(
                                            "{}\n\n... (truncated, original size: {} bytes)",
                                            &result_str[..50_000],
                                            result_str.len()
                                        )
                                    } else {
                                        result_str
                                    };

                                    // Normal successful response
                                    success_response(
                                        request_id.clone(),
                                        json!({
                                            "content": [{
                                                "type": "text",
                                                "text": trimmed_result
                                            }]
                                        }),
                                    )
                                }
                            }
                            Err(e) => error_response(
                                request_id.clone(),
                                INTERNAL_ERROR,
                                format!("Tool execution error: {}", e),
                                None,
                            ),
                        }
                    } else {
                        error_response(
                            request_id.clone(),
                            INVALID_PARAMS,
                            "Invalid parameters".to_string(),
                            None,
                        )
                    }
                } else {
                    error_response(
                        request_id.clone(),
                        INVALID_PARAMS,
                        "Missing parameters".to_string(),
                        None,
                    )
                }
            }

            "tools/list" => {
                // Get dynamic tool list from server
                match mcp_server.get_tool_schemas() {
                    Ok(schemas) => {
                        let tools: Vec<Value> = schemas
                            .into_iter()
                            .map(|schema| {
                                json!({
                                    "name": schema.name,
                                    "description": schema.description,
                                    "inputSchema": schema.parameters
                                })
                            })
                            .collect();

                        success_response(
                            request_id.clone(),
                            json!({
                                "tools": tools,
                                "_meta": {
                                    "critical_setup": "MUST call setup_xcuitest with target_app_bundle_id BEFORE any text-based UI interaction! Without this, all text/accessibility_id taps will fail.",
                                    "workflow": "1) device_management to get device_id, 2) setup_xcuitest with device_id AND target_app_bundle_id of the app you want to test, 3) Then use ui_interaction with text targets",
                                    "xcuitest_status": "Text-based element finding requires XCUITest initialization via setup_xcuitest tool WITH target_app_bundle_id parameter.",
                                    "setup_example": "{\"tool\": \"setup_xcuitest\", \"arguments\": {\"device_id\": \"YOUR-DEVICE-ID\", \"target_app_bundle_id\": \"com.example.app\"}}",
                                    "usage_hint": "Call usage_guide tool for detailed iOS automation guidance and examples."
                                }
                            }),
                        )
                    }
                    Err(e) => error_response(
                        request_id.clone(),
                        INTERNAL_ERROR,
                        format!("Failed to get tool schemas: {}", e),
                        None,
                    ),
                }
            }

            _ => error_response(
                request_id.clone(),
                METHOD_NOT_FOUND,
                format!("Method not found: {}", request.method),
                None,
            ),
        };

        // Validate response before sending
        let response_json = serde_json::to_value(&response)?;
        if let Err(e) = validate_response(&response_json) {
            eprintln!("ERROR: Generated invalid response: {}", e);
            eprintln!(
                "Response was: {}",
                serde_json::to_string_pretty(&response_json)?
            );

            // Send internal error instead
            let error_resp = error_response(
                match &response {
                    JsonRpcResponse::Success(s) => s.id.clone(),
                    JsonRpcResponse::Error(e) => e.id.clone(),
                },
                INTERNAL_ERROR,
                "Internal server error: Invalid response generated".to_string(),
                None,
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
}
