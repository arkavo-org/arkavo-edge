use arkavo_mcp_identity::McpIdentitySession;
use arkavo_mcp_macos::TestHarness;
use arkavo_mcp_macos::mcp::server::ToolRequest;
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
            .map(|e| format!("{}: {}", e.instance_path(), e))
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
            .map(|e| format!("{}: {}", e.instance_path(), e))
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

const REDACTED: &str = "[REDACTED]";
const MAX_LOG_PAYLOAD_CHARS: usize = 2048;
const SENSITIVE_KEY_PATTERNS: &[&str] = &[
    "api_key",
    "apikey",
    "token",
    "secret",
    "password",
    "authorization",
    "auth",
    "cookie",
    "session",
    "private_key",
    "bearer",
];

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SENSITIVE_KEY_PATTERNS
        .iter()
        .any(|pattern| lower.contains(pattern))
}

fn redact_value_for_log(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut redacted = serde_json::Map::with_capacity(map.len());
            for (key, val) in map {
                if is_sensitive_key(key) {
                    redacted.insert(key.clone(), Value::String(REDACTED.to_string()));
                } else {
                    redacted.insert(key.clone(), redact_value_for_log(val));
                }
            }
            Value::Object(redacted)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_value_for_log).collect()),
        _ => value.clone(),
    }
}

fn truncate_for_log(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{truncated}... [truncated, {count} chars total]")
}

fn sanitize_json_line_for_log(line: &str) -> String {
    match serde_json::from_str::<Value>(line) {
        Ok(value) => sanitize_json_value_for_log(&value),
        Err(_) => truncate_for_log("[non-json payload redacted]", MAX_LOG_PAYLOAD_CHARS),
    }
}

fn sanitize_json_value_for_log(value: &Value) -> String {
    let redacted = redact_value_for_log(value);
    let serialized = serde_json::to_string(&redacted).unwrap_or_else(|_| "{}".to_string());
    truncate_for_log(&serialized, MAX_LOG_PAYLOAD_CHARS)
}

#[allow(clippy::missing_panics_doc)]
pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize schemas
    init_schemas();

    // Initialize test harness
    let harness = TestHarness::new()
        .map_err(|e| anyhow::anyhow!("Failed to initialize test harness: {e}"))?;

    let mcp_server = harness.mcp_server();

    // Initialize MCP-I identity session for signed proofs
    let mcpi_session = McpIdentitySession::from_device_keypair()
        .unwrap_or_else(|_| McpIdentitySession::ephemeral());
    eprintln!("MCP-I session: did={}", mcpi_session.did());

    // Initialize memory tools
    if let Err(e) = mcp_server.initialize_memory_tools().await {
        eprintln!("Warning: Failed to initialize memory tools: {e}");
        // Continue without memory tools rather than failing completely
    }

    eprintln!("Arkavo MCP Server starting with schema validation...");

    // Set up panic handler to ensure clean JSON-RPC error on panic
    std::panic::set_hook(Box::new(|panic_info| {
        eprintln!("MCP Server panic: {panic_info:?}");
        // Don't output to stdout to avoid corrupting JSON-RPC stream
    }));

    // Main request/response loop
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    let reader = io::BufReader::new(stdin);
    for line in reader.lines() {
        let line = match line {
            Ok(l) => {
                eprintln!("MCP Server received: {}", sanitize_json_line_for_log(&l));
                l
            }
            Err(e) => {
                eprintln!("Error reading input: {e}");
                continue;
            }
        };

        // Parse as JSON first
        let json_value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("JSON parse error: {e}");
                // For parse errors, we can't send a proper error response
                // because we don't have a request ID
                continue;
            }
        };

        // Validate request schema
        if let Err(e) = validate_request(&json_value) {
            eprintln!("Request validation error: {e}");

            // Try to extract ID for error response
            if let Some(id) = json_value.get("id")
                && let Ok(id_val) = serde_json::from_value::<JsonRpcId>(id.clone())
            {
                let error_resp = error_response(
                    id_val,
                    INVALID_REQUEST,
                    format!("Invalid request: {e}"),
                    None,
                );

                let resp_json = serde_json::to_value(&error_resp)?;
                if let Err(e) = validate_response(&resp_json) {
                    eprintln!("ERROR: Generated invalid error response: {e}");
                    continue;
                }

                writeln!(stdout, "{}", serde_json::to_string(&error_resp)?)?;
                stdout.flush()?;
            }
            continue;
        }

        // Parse into typed request
        let request: JsonRpcRequest = match serde_json::from_value(json_value) {
            Ok(req) => req,
            Err(e) => {
                eprintln!("Failed to parse request: {e}");
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
                    "protocolVersion": "2025-11-25",
                    "serverInfo": {
                        "name": "arkavo",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": {
                        "tools": {}
                    },
                    "instructions": "This MCP server provides iOS automation with direct device control. ALWAYS USE COORDINATE-BASED TAPPING!\n\n🎯 PRIMARY METHOD - COORDINATE TAPPING (RECOMMENDED):\n- Use {\"action\":\"tap\",\"target\":{\"x\":200,\"y\":400}} - Works immediately!\n- No setup required - just use coordinates from screenshots\n- Most reliable method with embedded idb_companion\n\n📱 WORKFLOW:\n1. device_management to get device_id\n2. screen_capture to see the UI\n3. Identify element positions in the screenshot\n4. ui_interaction with {\"target\":{\"x\":X,\"y\":Y}}\n\n📏 DEVICE SIZES (logical points):\n- iPhone 15/16 Pro: 393×852\n- iPhone 16 Pro Max: 430×932\n- iPhone 15/16: 390×844\n\n⚠️ AVOID TEXT-BASED TAPPING:\n- Text tapping requires setup_xcuitest which often fails/timeouts\n- Even when it works, it's slower than coordinates\n- Only use as absolute last resort\n\n✅ ALWAYS prefer coordinates from screenshots over text-based element finding!"
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
                                    if error_obj.is_null() {
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

                                        // Normal successful response with MCP-I proof
                                        let meta = mcpi_session
                                            .sign_response(tool_name, &tool_response.result);
                                        success_response(
                                            request_id.clone(),
                                            json!({
                                                "content": [{
                                                    "type": "text",
                                                    "text": trimmed_result
                                                }],
                                                "_meta": meta,
                                            }),
                                        )
                                    } else {
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
                                            format!("{error_code}: {error_msg}"),
                                            Some(tool_response.result),
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

                                    // Normal successful response with MCP-I proof
                                    let meta = mcpi_session
                                        .sign_response(tool_name, &tool_response.result);
                                    success_response(
                                        request_id.clone(),
                                        json!({
                                            "content": [{
                                                "type": "text",
                                                "text": trimmed_result
                                            }],
                                            "_meta": meta,
                                        }),
                                    )
                                }
                            }
                            Err(e) => error_response(
                                request_id.clone(),
                                INTERNAL_ERROR,
                                format!("Tool execution error: {e}"),
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
                                    "🎯 ALWAYS_USE_COORDINATES": "Coordinate tapping {\"x\":200,\"y\":400} is the PRIMARY method. Works instantly!",
                                    "🚫 AVOID_TEXT_TAPPING": "Text-based tapping requires setup_xcuitest which OFTEN FAILS with timeouts. DO NOT USE unless absolutely necessary!",
                                    "✅ CORRECT_WORKFLOW": [
                                        "1. device_management → get device_id",
                                        "2. screen_capture → see UI visually",
                                        "3. Read the screenshot image",
                                        "4. ui_interaction with {\"target\":{\"x\":X,\"y\":Y}}"
                                    ],
                                    "📏 SCREEN_SIZES": {
                                        "iPhone_15_16_Pro": "393×852 points",
                                        "iPhone_16_Pro_Max": "430×932 points",
                                        "iPhone_15_16": "390×844 points"
                                    },
                                    "💡 EXAMPLE": {
                                        "tool": "ui_interaction",
                                        "arguments": {
                                            "action": "tap",
                                            "target": {"x": 196, "y": 426},
                                            "device_id": "YOUR-DEVICE-ID"
                                        }
                                    },
                                    "⚠️ LAST_RESORT_ONLY": "setup_xcuitest is unreliable and should be avoided. Coordinates work better!"
                                }
                            }),
                        )
                    }
                    Err(e) => error_response(
                        request_id.clone(),
                        INTERNAL_ERROR,
                        format!("Failed to get tool schemas: {e}"),
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
            eprintln!("ERROR: Generated invalid response: {e}");
            eprintln!(
                "Redacted response was: {}",
                sanitize_json_value_for_log(&response_json)
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
        eprintln!(
            "MCP Server sending response: {}",
            sanitize_json_value_for_log(&response_json)
        );
        writeln!(stdout, "{response_str}")?;
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

    #[test]
    fn test_sanitize_json_line_for_log_redacts_nested_sensitive_fields() {
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"tool":"x","api_key":"sk-123","nested":{"token":"abc","password":"p@ss"}}}"#;
        let sanitized = sanitize_json_line_for_log(line);

        assert!(!sanitized.contains("sk-123"));
        assert!(!sanitized.contains("\"abc\""));
        assert!(!sanitized.contains("p@ss"));
        assert!(sanitized.contains(REDACTED));
        assert!(sanitized.contains("tools/call"));
    }

    #[test]
    fn test_sanitize_json_value_for_log_redacts_response_result_and_error_data() {
        let value = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "access_token": "tok-xyz",
                "payload": "ok"
            },
            "error": {
                "code": -32603,
                "message": "boom",
                "data": {
                    "secret": "super-secret"
                }
            }
        });

        let sanitized = sanitize_json_value_for_log(&value);
        assert!(!sanitized.contains("tok-xyz"));
        assert!(!sanitized.contains("super-secret"));
        assert!(sanitized.contains(REDACTED));
        assert!(sanitized.contains("\"payload\":\"ok\""));
    }
}
