#![allow(clippy::disallowed_methods)]
#![allow(clippy::field_reassign_with_default)]

use arkavo_protocol::config::ServerConfig;
use arkavo_protocol::{generate_openrpc_schema, openrpc_to_json};
use arkavo_server::A2aServer;

#[tokio::test]
async fn test_openrpc_schema_generation() {
    let schema = generate_openrpc_schema();

    // Verify basic structure
    assert_eq!(schema.openrpc, "1.2.6");
    assert_eq!(schema.info.title, "Arkavo A2A Transport Protocol");
    assert!(!schema.info.version.is_empty());

    // Verify methods exist
    assert!(schema.methods.len() >= 3);
    let method_names: Vec<&str> = schema.methods.iter().map(|m| m.name.as_str()).collect();

    assert!(method_names.contains(&"task_request"));
    assert!(method_names.contains(&"task_declare"));
    assert!(method_names.contains(&"agent_discover"));

    // Verify components
    assert!(schema.components.is_some());

    // Verify servers
    assert!(schema.servers.is_some());
    assert!(!schema.servers.as_ref().unwrap().is_empty());
}

#[tokio::test]
async fn test_openrpc_json_export() {
    let json = openrpc_to_json().unwrap();

    // Verify it's valid JSON
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    // Verify key fields
    assert!(parsed.get("openrpc").is_some());
    assert!(parsed.get("info").is_some());
    assert!(parsed.get("methods").is_some());
}

#[tokio::test]
async fn test_server_with_openrpc() {
    let mut config = ServerConfig::default();
    config.port = 0; // Use random port
    config.enabled = true;
    config.task_store_path = None; // Use in-memory database for tests

    let _server = A2aServer::new(config);

    // The server should have the rpc.discover method available
    // This is tested indirectly through the unit tests
    // A full integration test would require starting the server
    // and making an actual JSON-RPC call
}

#[tokio::test]
async fn test_openrpc_compliance() {
    let schema = generate_openrpc_schema();

    // Verify OpenRPC 1.2.6 compliance
    assert_eq!(schema.openrpc, "1.2.6");

    // Each method should have required fields
    for method in &schema.methods {
        assert!(!method.name.is_empty());
        assert!(!method.params.is_empty() || method.name == "rpc.discover");
        assert!(!method.result.name.is_empty());
    }

    // Verify error codes follow JSON-RPC 2.0 spec
    for method in &schema.methods {
        if let Some(errors) = &method.errors {
            for error in errors {
                // Standard JSON-RPC error codes
                assert!(
                    error.code == -32700  // Parse error
                    || error.code == -32600    // Invalid Request
                    || error.code == -32601    // Method not found
                    || error.code == -32602    // Invalid params
                    || error.code == -32603    // Internal error
                    || (error.code >= -32099 && error.code <= -32000)
                ); // Server error
            }
        }
    }
}
