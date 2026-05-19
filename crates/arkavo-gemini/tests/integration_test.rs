//! Integration tests for arkavo-gemini
//!
//! These tests use #[tokio::test] which internally uses Runtime::block_on.
#![allow(clippy::disallowed_methods)]

use arkavo_gemini::{LiveSessionClient, ToolDispatcher, ToolRegistry};
use serde_json::json;

#[tokio::test]
async fn test_client_creation() {
    let client = LiveSessionClient::new("test_api_key", "gemini-3.5-flash");
    assert!(!client.is_connected());
}

#[tokio::test]
async fn test_client_with_tools() {
    let dispatcher = ToolDispatcher::new(2);
    let mut registry = ToolRegistry::new();

    registry.register(
        "test_tool",
        "A test tool",
        json!({"type": "object", "required": ["param1"]}),
        |_args| Ok(json!({"result": "success"})),
    );
    registry.build(&dispatcher);

    let tools = dispatcher.list_tools();
    let client = LiveSessionClient::new_with_tools("test_api_key", "gemini-3.5-flash", tools);
    assert!(!client.is_connected());
}

#[tokio::test]
async fn test_dispatcher_with_tools() {
    let dispatcher = ToolDispatcher::new(4);
    let mut registry = ToolRegistry::new();

    registry.register(
        "create_stream",
        "Creates a new stream",
        json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "openness": {"type": "string"}
            },
            "required": ["name", "openness"]
        }),
        |args| {
            let name = args["name"].as_str().unwrap();
            Ok(json!({
                "id": "stream-123",
                "message": format!("Stream '{}' created successfully", name)
            }))
        },
    );

    registry.build(&dispatcher);

    let tools = dispatcher.list_tools();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "create_stream");
}

#[tokio::test]
async fn test_tool_execution() {
    let dispatcher = ToolDispatcher::new(2);
    let mut registry = ToolRegistry::new();

    registry.register(
        "echo",
        "Echoes input",
        json!({"type": "object", "required": ["message"]}),
        |args| Ok(json!({"echo": args["message"]})),
    );

    registry.build(&dispatcher);

    let calls = vec![arkavo_gemini::FunctionCall {
        id: "call-1".to_string(),
        name: "echo".to_string(),
        args: json!({"message": "hello"}),
    }];

    let results = dispatcher.dispatch(calls).await;
    assert_eq!(results.len(), 1);
    assert!(results[0].1.is_ok());
}
