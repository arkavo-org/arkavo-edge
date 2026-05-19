//! Live API test example
//!
//! This example uses #[tokio::main] which internally uses Runtime::block_on.
#![allow(clippy::disallowed_methods)]

use arkavo_gemini::{LiveModality, LiveSessionClient, ToolDispatcher, ToolRegistry};
use serde_json::json;
use std::env;
use tokio::time::{Duration, sleep};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let api_key = env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY required");
    // Live API (bidiGenerateContent) requires a Live-capable model. As of
    // May 2026 only `gemini-2.5-flash-native-audio-*` is exposed via AI Studio
    // for v1beta WebSocket sessions; the gemini-3.1-flash-live helper from
    // the 3.5 launch is gated to Vertex AI / preview accounts. Override with
    // `GEMINI_LIVE_MODEL` if your account has 3.1-flash-live available.
    let model = env::var("GEMINI_LIVE_MODEL")
        .unwrap_or_else(|_| "gemini-2.5-flash-native-audio-latest".to_string());
    println!("Using Live API model: {model}");

    println!("Setting up tool dispatcher...");
    let dispatcher = ToolDispatcher::new(2);
    let mut registry = ToolRegistry::new();

    registry.register(
        "create_stream",
        "Creates a new stream with specified name and openness level",
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The unique name for the stream"
                },
                "openness": {
                    "type": "string",
                    "enum": ["PreApproved", "Apply", "Open"],
                    "description": "Access control policy for the stream"
                }
            },
            "required": ["name", "openness"]
        }),
        |args| {
            let name = args["name"].as_str().unwrap_or("unknown");
            let openness = args["openness"].as_str().unwrap_or("unknown");
            println!("✓ Tool executed: create_stream(name={name}, openness={openness})");
            Ok(json!({
                "id": format!("stream-{}", uuid::Uuid::new_v4()),
                "name": name,
                "openness": openness,
                "status": "created",
                "message": format!("Stream '{}' created successfully with {} access", name, openness)
            }))
        },
    );

    registry.build(&dispatcher);
    let tool_schemas = dispatcher.list_tools();

    println!("✓ Registered {} tools", tool_schemas.len());
    println!("\nCreating Live API client with tools...");

    // Native-audio models only emit AUDIO; flip modality accordingly.
    let client = LiveSessionClient::new_with_tools(api_key, &model, tool_schemas)
        .with_modality(LiveModality::Audio);

    println!("Connecting to Gemini Live API...");
    client.connect().await?;
    println!("✓ Connected successfully!\n");

    sleep(Duration::from_millis(500)).await;

    println!("Sending prompt with tool request...");
    client
        .send_prompt("Please create a new stream called 'release-canary' and make it Open")
        .await?;
    println!("✓ Prompt sent\n");

    println!("Waiting for response (15s timeout)...");
    tokio::select! {
        result = client.receive_tool_calls() => {
            match result {
                Ok(calls) => {
                    println!("✓ Received {} tool call(s)!\n", calls.len());
                    for call in &calls {
                        println!("  Function: {}", call.name);
                        println!("  Args: {}", serde_json::to_string_pretty(&call.args)?);
                        println!("  ID: {}\n", call.id);
                    }

                    println!("Dispatching tools...");
                    let results = dispatcher.dispatch(calls).await;

                    println!("\nSending tool responses...");
                    for (id, result) in results {
                        match result {
                            Ok(value) => {
                                println!("✓ Tool {id} succeeded");
                                client.send_tool_response(&id, value).await?;
                            }
                            Err(e) => {
                                println!("✗ Tool {id} failed: {e}");
                                client.send_tool_response(&id, json!({"error": e.to_string()})).await?;
                            }
                        }
                    }
                    println!("\n✓ All tool responses sent!");
                }
                Err(e) => {
                    println!("✗ Error receiving tool calls: {e}");
                }
            }
        }
        _ = sleep(Duration::from_secs(15)) => {
            println!("✗ Timeout waiting for response");
        }
    }

    println!("\nClosing connection...");
    client.close().await?;
    println!("✓ Test complete!");

    Ok(())
}
