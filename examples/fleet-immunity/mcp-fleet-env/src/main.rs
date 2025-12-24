//! Fleet Environment MCP Server
//!
//! Two modes:
//! - `--serve --port 8360` - HTTP server with shared state
//! - `--connect http://localhost:8360` - MCP stdio proxy to HTTP server

use arkavo_mcp_fleet_env::{FleetEnvState, GetSectorTool, Hazard, InjectHazardTool, Tool};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::sync::Arc;

const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;
const INTERNAL_ERROR: i32 = -32603;

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

#[derive(Debug, Deserialize)]
struct InjectHazardRequest {
    sector: u8,
    hazard_type: String,
    traction: f32,
}

fn success_response(id: JsonRpcId, result: Value) -> JsonRpcResponse {
    JsonRpcResponse::Success(JsonRpcSuccessResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result,
    })
}

fn error_response(id: JsonRpcId, code: i32, message: String) -> JsonRpcResponse {
    JsonRpcResponse::Error(JsonRpcErrorResponse {
        jsonrpc: "2.0".to_string(),
        id,
        error: JsonRpcError {
            code,
            message,
            data: None,
        },
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    let mut serve_mode = false;
    let mut port: u16 = 8360;
    let mut connect_url: Option<String> = None;
    let mut sector_count: u8 = 4;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--serve" => {
                serve_mode = true;
                i += 1;
            }
            "--port" if i + 1 < args.len() => {
                port = args[i + 1].parse().unwrap_or(8360);
                i += 2;
            }
            "--connect" if i + 1 < args.len() => {
                connect_url = Some(args[i + 1].clone());
                i += 2;
            }
            "--sector-count" if i + 1 < args.len() => {
                sector_count = args[i + 1].parse().unwrap_or(4);
                i += 2;
            }
            _ => i += 1,
        }
    }

    if serve_mode {
        run_http_server(port, sector_count).await
    } else if let Some(url) = connect_url {
        run_mcp_proxy(&url).await
    } else {
        // Default: standalone MCP server (original behavior)
        run_standalone_mcp(sector_count).await
    }
}

/// HTTP server mode - shared state for all rovers
async fn run_http_server(port: u16, sector_count: u8) -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(FleetEnvState::new(sector_count));

    let app = Router::new()
        .route("/sector/{id}", get(get_sector_handler))
        .route("/hazard", post(inject_hazard_handler))
        .route("/health", get(health_handler))
        .with_state(state);

    let addr = format!("0.0.0.0:{port}");
    eprintln!("Fleet Environment HTTP Server listening on {addr}");
    eprintln!("  GET  /sector/:id  - Query sector info");
    eprintln!("  POST /hazard      - Inject hazard");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health_handler() -> &'static str {
    "ok"
}

async fn get_sector_handler(
    State(state): State<Arc<FleetEnvState>>,
    Path(id): Path<u8>,
) -> Result<Json<Value>, StatusCode> {
    match state.get_sector(id).await {
        Some(sector) => Ok(Json(json!({
            "id": sector.id,
            "name": sector.name,
            "hazard": sector.hazard.map(|h| json!({
                "type": h.hazard_type,
                "traction": h.traction
            }))
        }))),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn inject_hazard_handler(
    State(state): State<Arc<FleetEnvState>>,
    Json(req): Json<InjectHazardRequest>,
) -> Result<Json<Value>, StatusCode> {
    let hazard = Hazard {
        hazard_type: req.hazard_type.clone(),
        traction: req.traction,
    };

    if state.inject_hazard(req.sector, hazard).await {
        eprintln!(
            "[HAZARD] Injected {} into Sector {} (traction: {})",
            req.hazard_type, req.sector, req.traction
        );
        Ok(Json(json!({
            "success": true,
            "sector": req.sector,
            "hazard_type": req.hazard_type,
            "traction": req.traction
        })))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// MCP proxy mode - forwards tool calls to HTTP server
async fn run_mcp_proxy(server_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let base_url = server_url.trim_end_matches('/').to_string();

    eprintln!("Fleet Environment MCP Proxy connecting to {base_url}");

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let reader = io::BufReader::new(stdin);

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Error reading input: {e}");
                continue;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(e) => {
                eprintln!("JSON parse error: {e}");
                continue;
            }
        };

        if request.id.is_none() {
            continue;
        }

        let request_id = request.id.unwrap();

        let response = match request.method.as_str() {
            "initialize" => success_response(
                request_id,
                json!({
                    "protocolVersion": "2024-11-05",
                    "serverInfo": { "name": "fleet-env-proxy", "version": "0.1.0" },
                    "capabilities": { "tools": {} },
                    "instructions": "Fleet environment proxy. Use get_sector and inject_hazard tools."
                }),
            ),

            "tools/list" => success_response(
                request_id,
                json!({
                    "tools": [
                        {
                            "name": "get_sector",
                            "description": "Get information about a warehouse sector",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "id": { "type": "integer", "description": "Sector ID" }
                                },
                                "required": ["id"]
                            }
                        },
                        {
                            "name": "inject_hazard",
                            "description": "Inject a hazard into a sector",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "sector": { "type": "integer" },
                                    "hazard_type": { "type": "string" },
                                    "traction": { "type": "number" }
                                },
                                "required": ["sector", "hazard_type", "traction"]
                            }
                        }
                    ]
                }),
            ),

            "tools/call" => {
                if let Some(params) = request.params {
                    let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let args = params.get("arguments").cloned().unwrap_or(json!({}));

                    let result = match tool_name {
                        "get_sector" => {
                            let id = args.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                            let url = format!("{base_url}/sector/{id}");
                            match client.get(&url).send().await {
                                Ok(resp) if resp.status().is_success() => {
                                    resp.json::<Value>().await.ok()
                                }
                                _ => None,
                            }
                        }
                        "inject_hazard" => {
                            let url = format!("{base_url}/hazard");
                            match client.post(&url).json(&args).send().await {
                                Ok(resp) if resp.status().is_success() => {
                                    resp.json::<Value>().await.ok()
                                }
                                _ => None,
                            }
                        }
                        _ => None,
                    };

                    match result {
                        Some(value) => {
                            let text = serde_json::to_string_pretty(&value)?;
                            success_response(
                                request_id,
                                json!({ "content": [{ "type": "text", "text": text }] }),
                            )
                        }
                        None => {
                            error_response(request_id, INTERNAL_ERROR, "Tool call failed".into())
                        }
                    }
                } else {
                    error_response(request_id, INVALID_PARAMS, "Missing params".into())
                }
            }

            _ => error_response(request_id, METHOD_NOT_FOUND, format!("Unknown: {}", request.method)),
        };

        let response_str = serde_json::to_string(&response)?;
        writeln!(stdout, "{response_str}")?;
        stdout.flush()?;
    }

    Ok(())
}

/// Standalone MCP server (original behavior for testing)
async fn run_standalone_mcp(sector_count: u8) -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(FleetEnvState::new(sector_count));
    let get_sector = GetSectorTool::new(Arc::clone(&state));
    let inject_hazard = InjectHazardTool::new(Arc::clone(&state));

    eprintln!("Fleet Environment MCP Server (standalone) with {sector_count} sectors");

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let reader = io::BufReader::new(stdin);

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Error reading input: {e}");
                continue;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(_) => continue,
        };

        if request.id.is_none() {
            continue;
        }

        let request_id = request.id.unwrap();

        let response = match request.method.as_str() {
            "initialize" => success_response(
                request_id,
                json!({
                    "protocolVersion": "2024-11-05",
                    "serverInfo": { "name": "fleet-env", "version": "0.1.0" },
                    "capabilities": { "tools": {} }
                }),
            ),

            "tools/list" => {
                let tools = vec![
                    json!({
                        "name": get_sector.schema().name,
                        "description": get_sector.schema().description,
                        "inputSchema": get_sector.schema().parameters
                    }),
                    json!({
                        "name": inject_hazard.schema().name,
                        "description": inject_hazard.schema().description,
                        "inputSchema": inject_hazard.schema().parameters
                    }),
                ];
                success_response(request_id, json!({ "tools": tools }))
            }

            "tools/call" => {
                if let Some(params) = request.params {
                    let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let args = params.get("arguments").cloned().unwrap_or(json!({}));

                    let result = match tool_name {
                        "get_sector" => get_sector.execute(args).await,
                        "inject_hazard" => inject_hazard.execute(args).await,
                        _ => Err(format!("Unknown tool: {tool_name}").into()),
                    };

                    match result {
                        Ok(value) => {
                            let text = serde_json::to_string_pretty(&value)?;
                            success_response(
                                request_id,
                                json!({ "content": [{ "type": "text", "text": text }] }),
                            )
                        }
                        Err(e) => error_response(request_id, INTERNAL_ERROR, e.to_string()),
                    }
                } else {
                    error_response(request_id, INVALID_PARAMS, "Missing params".into())
                }
            }

            _ => error_response(request_id, METHOD_NOT_FOUND, format!("Unknown: {}", request.method)),
        };

        let response_str = serde_json::to_string(&response)?;
        writeln!(stdout, "{response_str}")?;
        stdout.flush()?;
    }

    Ok(())
}
