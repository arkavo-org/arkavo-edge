//! Critical traceability tests for the mcp-runtime component.

#![allow(clippy::disallowed_methods)]

use arkavo_mcp_runtime::tools::EchoTool;
use arkavo_mcp_runtime::{McpRuntime, McpServer, McpServerConfig, RpcRequest, ToolRequest};
use arkavo_test_macros::spec;
use std::path::Path;
use std::sync::Arc;

#[spec("MCPR-001")]
#[tokio::test]
async fn mcp_server_creation_initializes_state_and_serves_requests() {
    let server = McpServer::new();

    // A new server starts with an empty tool registry.
    assert!(server.list_tools().await.is_empty());

    // Once a tool is registered the server can list, describe, and execute it.
    server
        .register_tool("echo".to_string(), Arc::new(EchoTool::new()))
        .await
        .expect("registering echo tool should succeed");

    let tool_names = server.list_tools().await;
    assert!(tool_names.contains(&"echo".to_string()));

    let schema = server
        .get_tool_schema("echo")
        .await
        .expect("echo schema should be available");
    assert_eq!(schema.get("name").and_then(|v| v.as_str()), Some("echo"));

    let response = server
        .execute_tool(ToolRequest {
            tool_name: "echo".to_string(),
            params: serde_json::json!({"msg": "hi"}),
        })
        .await;
    assert!(response.success);
    assert_eq!(response.tool_name, "echo");

    // The server also exposes tools through JSON-RPC, showing it is ready for connections.
    let rpc = RpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(serde_json::json!(1)),
        method: "list_tools".to_string(),
        params: None,
    };
    let rpc_response = server.handle_rpc_request(rpc).await;
    assert!(rpc_response.error.is_none());

    let result_tools = rpc_response
        .result
        .expect("list_tools should return a result")
        .as_array()
        .expect("list_tools result should be an array")
        .clone();
    assert!(result_tools.iter().any(|v| v.as_str() == Some("echo")));

    let ping = server
        .handle_rpc_request(RpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(2)),
            method: "ping".to_string(),
            params: None,
        })
        .await;
    assert!(ping.error.is_none());

    let call = server
        .handle_rpc_request(RpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(3)),
            method: "tools/call".to_string(),
            params: Some(serde_json::json!({"name": "echo", "arguments": {"msg": "hi"}})),
        })
        .await;
    assert!(call.error.is_none());

    let listed = server
        .handle_rpc_request(RpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(4)),
            method: "tools/list".to_string(),
            params: Some(serde_json::json!({})),
        })
        .await;
    assert!(listed.error.is_none());
    assert!(
        listed
            .result
            .as_ref()
            .and_then(|v| v.get("tools"))
            .and_then(|v| v.as_array())
            .is_some()
    );

    let note = server
        .handle_rpc_request(RpcRequest {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: "notifications/initialized".to_string(),
            params: None,
        })
        .await;
    assert!(note.error.is_none());
}

#[cfg(unix)]
#[spec("MCPR-002")]
#[tokio::test]
async fn mcp_runtime_accepts_client_connection_and_tracks_state() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_mcp_server.py");
    let config = McpServerConfig::stdio(
        "fake-mcp",
        "python3",
        vec!["-u".to_string(), fixture.to_string_lossy().to_string()],
    );

    let runtime = McpRuntime::new();

    let handle = runtime
        .add_server(config)
        .await
        .expect("add_server should connect to the fake MCP server");

    // A handle is returned and the runtime tracks the connection.
    assert_eq!(handle.name(), "fake-mcp");
    assert!(runtime.is_connected("fake-mcp").await);
    assert!(
        runtime
            .list_servers()
            .await
            .contains(&"fake-mcp".to_string())
    );

    // The remote tool was discovered and registered as a proxy tool in the local server.
    let server = runtime.server().await;
    let tool_names = server.list_tools().await;
    assert!(tool_names.contains(&"echo".to_string()));
    drop(server);

    // Removing the server cleans up the connection and its tools.
    runtime
        .remove_server("fake-mcp")
        .await
        .expect("removing server should succeed");
    assert!(!runtime.is_connected("fake-mcp").await);
}
