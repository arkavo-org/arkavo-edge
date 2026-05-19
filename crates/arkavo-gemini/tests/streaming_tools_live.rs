//! Live integration test: SSE `streamGenerateContent` with function calling.
//!
//! Skipped unless `GEMINI_API_KEY` is set. Verifies the production streaming
//! path: SSE chunks parsed, a `functionCall` part arrives, dispatcher
//! executes it. Asserts a sub-10s TTFT as a smoke-level perf guard.
#![allow(clippy::disallowed_methods)]

use arkavo_gemini::{
    FunctionDeclaration, GeminiSseStream, RestClient, ToolDispatcher, ToolRegistry,
};
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
async fn sse_streaming_invokes_tool() {
    let Some(api_key) = api_key_or_skip("sse_streaming_invokes_tool") else {
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
                "name": { "type": "string" },
                "openness": { "type": "string", "enum": ["PreApproved", "Apply", "Open"] }
            },
            "required": ["name", "openness"]
        }),
        |args| {
            Ok(json!({
                "id": format!("stream-{}", uuid::Uuid::new_v4()),
                "name": args["name"],
                "openness": args["openness"],
                "status": "created"
            }))
        },
    );
    registry.build(&dispatcher);

    let tools: Vec<FunctionDeclaration> = dispatcher
        .list_tools()
        .iter()
        .map(|t| FunctionDeclaration {
            name: t["name"].as_str().unwrap().to_string(),
            description: t["description"].as_str().unwrap().to_string(),
            parameters: t["parameters"].clone(),
        })
        .collect();

    let client = RestClient::new(api_key, &model);
    let start = std::time::Instant::now();
    let mut stream: GeminiSseStream = client
        .stream_generate_content(
            "Please create a new stream called 'release-canary' and make it Open",
            Some(tools),
            None,
        )
        .await
        .expect("stream_generate_content failed");

    let mut ttft = None;
    let mut tool_calls = Vec::new();
    let mut chunk_count = 0;

    while let Some(result) = stream.next().await {
        let response = result.expect("stream chunk error");
        chunk_count += 1;
        if ttft.is_none() {
            ttft = Some(start.elapsed());
        }
        if !response.function_calls.is_empty() {
            tool_calls.extend(response.function_calls.clone());
        }
        if response.done {
            break;
        }
    }

    assert!(chunk_count > 0, "no SSE chunks received");
    assert!(!tool_calls.is_empty(), "expected tool call in stream");
    assert_eq!(tool_calls[0].name, "create_stream");

    let ttft = ttft.expect("TTFT unset");
    assert!(
        ttft < std::time::Duration::from_secs(10),
        "TTFT regression: {ttft:?} exceeds 10s"
    );
}
