//! Live integration test: REST `generateContent` with function calling.
//!
//! Skipped unless `GEMINI_API_KEY` is set. Verifies the production REST
//! tool-calling path end-to-end: tool declaration → server returns
//! `functionCall` part → dispatcher executes registered handler.
#![allow(clippy::disallowed_methods)]

use arkavo_gemini::{FunctionDeclaration, RestClient, ToolDispatcher, ToolRegistry};
use serde_json::json;

fn api_key_or_skip(test: &str) -> Option<String> {
    match std::env::var("GEMINI_API_KEY") {
        Ok(k) if !k.is_empty() => Some(k),
        _ => {
            eprintln!("skipping {test}: GEMINI_API_KEY not set");
            None
        }
    }
}

#[tokio::test]
async fn rest_generate_content_invokes_tool() {
    let Some(api_key) = api_key_or_skip("rest_generate_content_invokes_tool") else {
        return;
    };
    let model = std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-3.5-flash".to_string());

    let dispatcher = ToolDispatcher::new(2);
    let mut registry = ToolRegistry::new();
    registry.register(
        "create_stream",
        "Creates a new stream with specified name and openness level",
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Unique stream name" },
                "openness": {
                    "type": "string",
                    "enum": ["PreApproved", "Apply", "Open"],
                    "description": "Access control policy"
                }
            },
            "required": ["name", "openness"]
        }),
        |args| {
            let name = args["name"].as_str().unwrap_or("unknown");
            let openness = args["openness"].as_str().unwrap_or("unknown");
            Ok(json!({
                "id": format!("stream-{}", uuid::Uuid::new_v4()),
                "name": name,
                "openness": openness,
                "status": "created"
            }))
        },
    );
    registry.build(&dispatcher);

    let tool_decls: Vec<FunctionDeclaration> = dispatcher
        .list_tools()
        .iter()
        .map(|t| FunctionDeclaration {
            name: t["name"].as_str().unwrap().to_string(),
            description: t["description"].as_str().unwrap().to_string(),
            parameters: t["parameters"].clone(),
        })
        .collect();

    let client = RestClient::new(api_key, model);
    let (_text, calls) = client
        .generate_content(
            "Please create a new stream called 'release-canary' and make it Open",
            Some(tool_decls),
            None,
        )
        .await
        .expect("generate_content failed");

    assert!(!calls.is_empty(), "expected at least one tool call");
    let call = &calls[0];
    assert_eq!(call.name, "create_stream");
    assert_eq!(call.args["name"].as_str(), Some("release-canary"));
    assert_eq!(call.args["openness"].as_str(), Some("Open"));

    let results = dispatcher.dispatch(calls).await;
    let (_id, result) = results.into_iter().next().expect("no dispatch result");
    let value = result.expect("dispatcher returned error");
    assert_eq!(value["status"].as_str(), Some("created"));
}
