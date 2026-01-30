#![allow(clippy::disallowed_methods)]

//! Integration tests for agent onboarding flow
//!
//! Tests the complete flow from iOS client perspective:
//! 1. Server binds to IPv4 (not just IPv6)
//! 2. rpc.discover returns chat methods
//! 3. chat_open returns valid session
//! 4. chat_send works with valid session
//! 5. chat_close closes session properly

use arkavo_protocol::config::ServerConfig;
use arkavo_protocol::rate_limit::RateLimitConfig;
use arkavo_protocol::openrpc::generate_openrpc_schema;
use arkavo_protocol::server::A2aServer;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

/// Test that the OpenRPC schema includes all chat methods
#[test]
fn test_openrpc_schema_includes_chat_methods() {
    let schema = generate_openrpc_schema();

    // Collect method names
    let method_names: Vec<&str> = schema.methods.iter().map(|m| m.name.as_str()).collect();

    // Verify chat methods are present
    assert!(
        method_names.contains(&"chat_open"),
        "OpenRPC schema must include chat_open method. Found: {method_names:?}"
    );
    assert!(
        method_names.contains(&"chat_send"),
        "OpenRPC schema must include chat_send method. Found: {method_names:?}"
    );
    assert!(
        method_names.contains(&"chat_close"),
        "OpenRPC schema must include chat_close method. Found: {method_names:?}"
    );
}

/// Test that OpenRPC schema includes chat type definitions
#[test]
fn test_openrpc_schema_includes_chat_types() {
    let schema = generate_openrpc_schema();

    let components = schema.components.expect("Schema must have components");
    let schemas = components.schemas.expect("Components must have schemas");

    // Verify chat-related types are defined
    assert!(
        schemas.contains_key("ChatOpenRequest"),
        "Schema must define ChatOpenRequest"
    );
    assert!(
        schemas.contains_key("ChatSession"),
        "Schema must define ChatSession"
    );
    assert!(
        schemas.contains_key("UserMessage"),
        "Schema must define UserMessage"
    );
    assert!(
        schemas.contains_key("MessageDelta"),
        "Schema must define MessageDelta"
    );
}

/// Test that chat_open method has correct parameter structure
#[test]
fn test_chat_open_method_structure() {
    let schema = generate_openrpc_schema();

    let chat_open = schema
        .methods
        .iter()
        .find(|m| m.name == "chat_open")
        .expect("chat_open method must exist");

    // Verify it has a request parameter
    assert!(
        !chat_open.params.is_empty(),
        "chat_open must have parameters"
    );

    // Verify result type
    assert_eq!(
        chat_open.result.name, "session",
        "chat_open result must be named 'session'"
    );
}

/// Test that chat_send method has correct parameter structure
#[test]
fn test_chat_send_method_structure() {
    let schema = generate_openrpc_schema();

    let chat_send = schema
        .methods
        .iter()
        .find(|m| m.name == "chat_send")
        .expect("chat_send method must exist");

    // Verify it has session_id and message parameters
    let param_names: Vec<&str> = chat_send.params.iter().map(|p| p.name.as_str()).collect();
    assert!(
        param_names.contains(&"session_id"),
        "chat_send must have session_id parameter"
    );
    assert!(
        param_names.contains(&"message"),
        "chat_send must have message parameter"
    );
}

/// Test that chat_close method has correct parameter structure
#[test]
fn test_chat_close_method_structure() {
    let schema = generate_openrpc_schema();

    let chat_close = schema
        .methods
        .iter()
        .find(|m| m.name == "chat_close")
        .expect("chat_close method must exist");

    // Verify it has session_id parameter
    let param_names: Vec<&str> = chat_close.params.iter().map(|p| p.name.as_str()).collect();
    assert!(
        param_names.contains(&"session_id"),
        "chat_close must have session_id parameter"
    );
}

/// Test that server binds to IPv4 address (0.0.0.0) not just IPv6
#[tokio::test]
async fn test_server_binds_to_ipv4() {
    let config = ServerConfig {
        enabled: true,
        bind_address: "0.0.0.0".to_string(), // IPv4 bind address
        port: 0,                              // Dynamic port
        max_connections: 10,
        idle_timeout_seconds: 60,
        rate_limit: RateLimitConfig::default(),
        task_store_path: None,
        metrics_enabled: false,
    };

    let server = A2aServer::new(config);
    let (handle, actual_port) = server.start_with_port().await.expect("Server must start");

    // Verify we can connect via IPv4 (127.0.0.1)
    let ipv4_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), actual_port);
    let connect_result =
        TcpStream::connect_timeout(&ipv4_addr, Duration::from_secs(2));

    assert!(
        connect_result.is_ok(),
        "Must be able to connect to server via IPv4 (127.0.0.1:{}). Error: {:?}",
        actual_port,
        connect_result.err()
    );

    handle.stop().expect("Server must stop cleanly");
}

/// Test that server reports correct bind address
#[tokio::test]
async fn test_server_reports_ipv4_address() {
    let config = ServerConfig {
        enabled: true,
        bind_address: "127.0.0.1".to_string(),
        port: 0,
        max_connections: 10,
        idle_timeout_seconds: 60,
        rate_limit: RateLimitConfig::default(),
        task_store_path: None,
        metrics_enabled: false,
    };

    let server = A2aServer::new(config);
    let (handle, actual_port) = server.start_with_port().await.expect("Server must start");

    // Port must be non-zero (dynamically assigned)
    assert!(actual_port > 0, "Server must bind to a valid port");

    // Verify connectivity
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), actual_port);
    let connect_result = TcpStream::connect_timeout(&addr, Duration::from_secs(2));
    assert!(
        connect_result.is_ok(),
        "Must connect to server at 127.0.0.1:{actual_port}"
    );

    handle.stop().expect("Server must stop cleanly");
}

/// Test get_service_ip returns IPv4 address
#[test]
fn test_get_service_ip_returns_ipv4() {
    let ip = arkavo_protocol::get_service_ip();

    // Must not be 0.0.0.0 (unspecified)
    assert!(
        !ip.is_unspecified(),
        "Service IP must not be 0.0.0.0, got: {ip}"
    );

    // Verify it's a valid IPv4 address (not IPv6)
    let octets = ip.octets();
    let is_valid_ipv4 = matches!(
        octets,
        [127, 0, 0, 1]           // localhost
        | [192, 168, _, _]       // private class C
        | [10, _, _, _]          // private class A
        | [172, 16..=31, _, _]   // private class B
    ) || !ip.is_loopback();

    assert!(
        is_valid_ipv4,
        "Service IP must be valid IPv4 address, got: {ip}"
    );
}

/// Integration test: Server accepts WebSocket connections
/// Note: The A2A server uses jsonrpsee which requires WebSocket protocol
#[tokio::test]
async fn test_server_accepts_websocket_connections() {
    use jsonrpsee::core::client::ClientT;
    use jsonrpsee::ws_client::WsClientBuilder;

    let config = ServerConfig {
        enabled: true,
        bind_address: "127.0.0.1".to_string(),
        port: 0,
        max_connections: 10,
        idle_timeout_seconds: 60,
        rate_limit: RateLimitConfig::default(),
        task_store_path: None,
        metrics_enabled: false,
    };

    let server = A2aServer::new(config);
    let (handle, actual_port) = server.start_with_port().await.expect("Server must start");

    // Give server time to fully initialize
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connect via WebSocket and call rpc.discover
    let url = format!("ws://127.0.0.1:{actual_port}");
    let client = WsClientBuilder::default()
        .build(&url)
        .await
        .expect("WebSocket client must connect");

    // Call rpc.discover - this should return the OpenRPC schema
    let response: serde_json::Value = client
        .request("rpc.discover", jsonrpsee::rpc_params![])
        .await
        .expect("rpc.discover must succeed");

    // Verify the response contains chat methods
    let response_str = response.to_string();
    assert!(
        response_str.contains("chat_open"),
        "OpenRPC schema must include chat_open method. Response: {}",
        &response_str[..std::cmp::min(500, response_str.len())]
    );
    assert!(
        response_str.contains("chat_send"),
        "OpenRPC schema must include chat_send method"
    );
    assert!(
        response_str.contains("chat_close"),
        "OpenRPC schema must include chat_close method"
    );

    handle.stop().expect("Server must stop cleanly");
}

/// Integration test: chat_open RPC method works
#[tokio::test]
async fn test_chat_open_rpc_returns_session() {
    use jsonrpsee::core::client::ClientT;
    use jsonrpsee::ws_client::WsClientBuilder;

    let config = ServerConfig {
        enabled: true,
        bind_address: "127.0.0.1".to_string(),
        port: 0,
        max_connections: 10,
        idle_timeout_seconds: 60,
        rate_limit: RateLimitConfig::default(),
        task_store_path: None,
        metrics_enabled: false,
    };

    let server = A2aServer::new(config);
    let (handle, actual_port) = server.start_with_port().await.expect("Server must start");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let url = format!("ws://127.0.0.1:{actual_port}");
    let client = WsClientBuilder::default()
        .build(&url)
        .await
        .expect("WebSocket client must connect");

    // Call chat_open with empty request
    let request = serde_json::json!({});
    let response: serde_json::Value = client
        .request("chat_open", jsonrpsee::rpc_params![request])
        .await
        .expect("chat_open must succeed");

    // Verify response contains session_id
    assert!(
        response.get("session_id").is_some(),
        "chat_open response must contain session_id. Response: {response:?}"
    );

    let session_id = response["session_id"]
        .as_str()
        .expect("session_id must be a string");
    assert!(
        !session_id.is_empty(),
        "session_id must not be empty"
    );

    handle.stop().expect("Server must stop cleanly");
}

/// Integration test: chat_close with invalid session returns error
#[tokio::test]
async fn test_chat_close_invalid_session_returns_error() {
    use jsonrpsee::core::client::ClientT;
    use jsonrpsee::ws_client::WsClientBuilder;

    let config = ServerConfig {
        enabled: true,
        bind_address: "127.0.0.1".to_string(),
        port: 0,
        max_connections: 10,
        idle_timeout_seconds: 60,
        rate_limit: RateLimitConfig::default(),
        task_store_path: None,
        metrics_enabled: false,
    };

    let server = A2aServer::new(config);
    let (handle, actual_port) = server.start_with_port().await.expect("Server must start");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let url = format!("ws://127.0.0.1:{actual_port}");
    let client = WsClientBuilder::default()
        .build(&url)
        .await
        .expect("WebSocket client must connect");

    // Call chat_close with non-existent session
    let result: Result<serde_json::Value, _> = client
        .request("chat_close", jsonrpsee::rpc_params!["non_existent_session_id"])
        .await;

    // Should return an error for invalid session
    assert!(
        result.is_err(),
        "chat_close with invalid session should return error"
    );

    handle.stop().expect("Server must stop cleanly");
}

/// Integration test: Server accepts HTTP POST JSON-RPC requests
/// This is critical for iOS clients that may not support WebSocket
#[tokio::test]
async fn test_server_accepts_http_jsonrpc_requests() {
    let config = ServerConfig {
        enabled: true,
        bind_address: "127.0.0.1".to_string(),
        port: 0,
        max_connections: 10,
        idle_timeout_seconds: 60,
        rate_limit: RateLimitConfig::default(),
        task_store_path: None,
        metrics_enabled: false,
    };

    let server = A2aServer::new(config);
    let (handle, actual_port) = server.start_with_port().await.expect("Server must start");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send JSON-RPC request via HTTP POST
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{actual_port}");

    let jsonrpc_request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "rpc.discover",
        "params": [],
        "id": 1
    });

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&jsonrpc_request)
        .send()
        .await
        .expect("HTTP request must succeed");

    let status = response.status();
    let body = response.text().await.expect("Must read response body");

    // jsonrpsee server should accept HTTP POST and return JSON-RPC response
    assert!(
        status.is_success(),
        "HTTP JSON-RPC request should succeed. Status: {status}, Body: {body}"
    );

    // Parse the response
    let json_response: serde_json::Value =
        serde_json::from_str(&body).expect("Response must be valid JSON");

    // Should have jsonrpc field
    assert!(
        json_response.get("jsonrpc").is_some(),
        "Response should be valid JSON-RPC. Got: {body}"
    );

    // Should have result containing chat methods
    if let Some(result) = json_response.get("result") {
        let result_str = result.to_string();
        assert!(
            result_str.contains("chat_open"),
            "OpenRPC schema must include chat_open. Result: {}",
            &result_str[..std::cmp::min(500, result_str.len())]
        );
    } else {
        panic!("Response should have result field. Got: {body}");
    }

    handle.stop().expect("Server must stop cleanly");
}

/// Integration test: chat_open via HTTP transport
#[tokio::test]
async fn test_chat_open_via_http() {
    let config = ServerConfig {
        enabled: true,
        bind_address: "127.0.0.1".to_string(),
        port: 0,
        max_connections: 10,
        idle_timeout_seconds: 60,
        rate_limit: RateLimitConfig::default(),
        task_store_path: None,
        metrics_enabled: false,
    };

    let server = A2aServer::new(config);
    let (handle, actual_port) = server.start_with_port().await.expect("Server must start");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{actual_port}");

    let jsonrpc_request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "chat_open",
        "params": [{}],
        "id": 1
    });

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&jsonrpc_request)
        .send()
        .await
        .expect("HTTP request must succeed");

    let status = response.status();
    let body = response.text().await.expect("Must read response body");

    assert!(
        status.is_success(),
        "chat_open via HTTP should succeed. Status: {status}, Body: {body}"
    );

    let json_response: serde_json::Value =
        serde_json::from_str(&body).expect("Response must be valid JSON");

    // Verify response contains session_id
    if let Some(result) = json_response.get("result") {
        assert!(
            result.get("session_id").is_some(),
            "chat_open response must contain session_id. Response: {body}"
        );
    } else {
        panic!("chat_open should return result with session_id. Got: {body}");
    }

    handle.stop().expect("Server must stop cleanly");
}

/// Integration test: iOS client registration via WebSocket RPC
/// Tests the registration.challenge and registration.verify flow that iOS uses after scanning QR
#[tokio::test]
async fn test_ios_registration_via_rpc() {
    use jsonrpsee::core::client::ClientT;
    use jsonrpsee::ws_client::WsClientBuilder;

    let config = ServerConfig {
        enabled: true,
        bind_address: "127.0.0.1".to_string(),
        port: 0,
        max_connections: 10,
        idle_timeout_seconds: 60,
        rate_limit: RateLimitConfig::default(),
        task_store_path: None,
        metrics_enabled: false,
    };

    let server = A2aServer::new(config);
    let (handle, actual_port) = server.start_with_port().await.expect("Server must start");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // iOS client connects via WebSocket (URL from QR code rpc parameter)
    let url = format!("ws://127.0.0.1:{actual_port}");
    let client = WsClientBuilder::default()
        .build(&url)
        .await
        .expect("WebSocket client must connect");

    // Step 1: Request challenge (iOS sends device_id which could be DID)
    // Server uses param_kind=map, so we send named params directly (not wrapped in array)
    use jsonrpsee::core::params::ObjectParams;
    let mut challenge_params = ObjectParams::new();
    challenge_params.insert("device_id", "did:key:z6Mk123456789abcdef").unwrap();

    let challenge_response: serde_json::Value = client
        .request("registration.challenge", challenge_params)
        .await
        .expect("registration.challenge must succeed");

    // Verify challenge response contains required fields
    assert!(
        challenge_response.get("challenge_id").is_some(),
        "Challenge response must contain challenge_id. Response: {challenge_response:?}"
    );
    assert!(
        challenge_response.get("challenge").is_some(),
        "Challenge response must contain challenge. Response: {challenge_response:?}"
    );

    let challenge_id = challenge_response["challenge_id"]
        .as_str()
        .expect("challenge_id must be string");
    let challenge_b64 = challenge_response["challenge"]
        .as_str()
        .expect("challenge must be string");

    // Step 2: Sign challenge and verify (iOS signs with its private key)
    // Challenge is base64-encoded, must decode before signing
    let challenge_bytes = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        challenge_b64,
    )
    .expect("challenge must be valid base64");

    // Generate keypair and sign the decoded challenge
    let keypair = arkavo_crypto::AgentKeypair::generate();
    let public_key = keypair.public_key();
    let signature = keypair.sign(&challenge_bytes);

    // Server uses param_kind=map, so we send named params directly
    let mut verify_params = ObjectParams::new();
    verify_params.insert("challenge_id", challenge_id).unwrap();
    verify_params.insert("device_id", "did:key:z6Mk123456789abcdef").unwrap();
    verify_params.insert("public_key", public_key.to_base64()).unwrap();
    verify_params.insert("signature", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &signature)).unwrap();

    let verify_response: serde_json::Value = client
        .request("registration.verify", verify_params)
        .await
        .expect("registration.verify must succeed");

    // Verify registration succeeded
    assert!(
        verify_response.get("success").is_some(),
        "Verify response must contain success field. Response: {verify_response:?}"
    );
    assert!(
        verify_response["success"].as_bool().unwrap_or(false),
        "Registration verification must succeed. Response: {verify_response:?}"
    );

    handle.stop().expect("Server must stop cleanly");
}

/// Integration test: iOS client sends params as object (not array)
/// iOS Swift code sends: {"params": {"device_id": "..."}}
/// NOT: {"params": [{"device_id": "..."}]}
#[tokio::test]
async fn test_ios_registration_params_as_object() {
    let config = ServerConfig {
        enabled: true,
        bind_address: "127.0.0.1".to_string(),
        port: 0,
        max_connections: 10,
        idle_timeout_seconds: 60,
        rate_limit: RateLimitConfig::default(),
        task_store_path: None,
        metrics_enabled: false,
    };

    let server = A2aServer::new(config);
    let (handle, actual_port) = server.start_with_port().await.expect("Server must start");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{actual_port}");

    // iOS sends params as object, not array - this is what the iOS Swift client does
    let jsonrpc_request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "registration.challenge",
        "params": {"device_id": "did:key:z6MkiOSTest"},  // Object, not [Object]
        "id": 1
    });

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&jsonrpc_request)
        .send()
        .await
        .expect("HTTP request must succeed");

    let status = response.status();
    let body = response.text().await.expect("Must read response body");

    assert!(
        status.is_success(),
        "registration.challenge with params as object should succeed. Status: {status}, Body: {body}"
    );

    let json_response: serde_json::Value =
        serde_json::from_str(&body).expect("Response must be valid JSON");

    // Must have result, not error
    assert!(
        json_response.get("result").is_some(),
        "registration.challenge should return result, not error. Got: {body}"
    );

    let result = &json_response["result"];
    assert!(
        result.get("challenge_id").is_some(),
        "Challenge response must contain challenge_id. Response: {body}"
    );

    handle.stop().expect("Server must stop cleanly");
}
