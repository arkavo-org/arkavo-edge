use arkavo_test::Result;
use arkavo_test::mcp::server::{McpTestServer, ToolRequest};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
use tracing_subscriber::EnvFilter;

#[derive(serde::Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(serde::Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Option<Value>,
    result: Option<Value>,
    error: Option<Value>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    eprintln!("Starting Arkavo Test MCP Server...");

    // Create the MCP server
    let server = McpTestServer::new()?;
    eprintln!("MCP Server initialized successfully");

    // Print available tools
    let schemas = server.get_tool_schemas()?;
    eprintln!("Available tools: {}", schemas.len());
    for schema in &schemas {
        eprintln!("  - {}: {}", schema.name, schema.description);
    }

    // Start JSON-RPC loop
    eprintln!("\nMCP Server ready. Listening for JSON-RPC requests on stdin...");

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        // Parse JSON-RPC request
        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(e) => {
                let error_response = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: None,
                    result: None,
                    error: Some(json!({
                        "code": -32700,
                        "message": format!("Parse error: {}", e)
                    })),
                };
                writeln!(stdout, "{}", serde_json::to_string(&error_response)?)?;
                stdout.flush()?;
                continue;
            }
        };

        // Handle the request
        let response = match request.method.as_str() {
            "tools/list" => {
                // Return list of available tools
                JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: Some(json!({
                        "tools": schemas.iter().map(|s| json!({
                            "name": s.name,
                            "description": s.description,
                            "inputSchema": s.parameters
                        })).collect::<Vec<_>>()
                    })),
                    error: None,
                }
            }
            "tools/call" => {
                // Call a tool
                if let Some(params) = request.params {
                    if let (Some(name), Some(arguments)) = (
                        params.get("name").and_then(|v| v.as_str()),
                        params.get("arguments"),
                    ) {
                        let tool_request = ToolRequest {
                            tool_name: name.to_string(),
                            params: arguments.clone(),
                        };

                        match server.call_tool(tool_request).await {
                            Ok(tool_response) => JsonRpcResponse {
                                jsonrpc: "2.0".to_string(),
                                id: request.id,
                                result: Some(json!({
                                    "content": [{
                                        "type": "text",
                                        "text": serde_json::to_string_pretty(&tool_response.result)?
                                    }]
                                })),
                                error: None,
                            },
                            Err(e) => JsonRpcResponse {
                                jsonrpc: "2.0".to_string(),
                                id: request.id,
                                result: None,
                                error: Some(json!({
                                    "code": -32603,
                                    "message": format!("Tool execution error: {}", e)
                                })),
                            },
                        }
                    } else {
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: request.id,
                            result: None,
                            error: Some(json!({
                                "code": -32602,
                                "message": "Invalid params: missing 'name' or 'arguments'"
                            })),
                        }
                    }
                } else {
                    JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: request.id,
                        result: None,
                        error: Some(json!({
                            "code": -32602,
                            "message": "Invalid params: params required"
                        })),
                    }
                }
            }
            _ => {
                // Method not found
                JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: None,
                    error: Some(json!({
                        "code": -32601,
                        "message": format!("Method not found: {}", request.method)
                    })),
                }
            }
        };

        // Send response
        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }

    Ok(())
}
