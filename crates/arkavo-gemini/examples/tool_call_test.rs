use arkavo_gemini::{LiveSessionClient, ToolDispatcher, ToolRegistry};
use serde_json::json;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let api_key = env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY required");
    let model = "models/gemini-2.0-flash-exp";

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
            println!("✓ Tool executed: create_stream(name={}, openness={})", name, openness);
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
    println!("✓ Registered {} tools", dispatcher.list_tools().len());

    let tool_schemas = dispatcher.list_tools();
    println!("\nCreating client with tools...");
    let client = LiveSessionClient::new_with_tools(api_key, model, tool_schemas);

    println!("Connecting to Gemini Live API...");
    client.connect().await?;
    println!("✓ Connected successfully!");

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    println!("\nSending prompt requesting tool call...");
    client
        .send_prompt("Please create a new stream called 'release-canary' and make it Open")
        .await?;
    println!("✓ Prompt sent");

    println!("\nWaiting for tool calls from Gemini (10s timeout)...");
    tokio::select! {
        result = client.receive_tool_calls() => {
            match result {
                Ok(calls) => {
                    println!("✓ Received {} tool call(s)!", calls.len());
                    for call in &calls {
                        println!("\n  Function: {}", call.name);
                        println!("  Args: {}", serde_json::to_string_pretty(&call.args)?);
                        println!("  ID: {}", call.id);
                    }

                    println!("\nDispatching tools...");
                    let results = dispatcher.dispatch(calls).await;

                    println!("\nSending tool responses...");
                    for (id, result) in results {
                        match result {
                            Ok(value) => {
                                println!("✓ Tool {} succeeded", id);
                                println!("  Result: {}", serde_json::to_string_pretty(&value)?);
                                client.send_tool_response(&id, value).await?;
                            }
                            Err(e) => {
                                println!("✗ Tool {} failed: {}", id, e);
                                client.send_tool_response(&id, json!({"error": e.to_string()})).await?;
                            }
                        }
                    }
                    println!("\n✓ All tool responses sent!");
                }
                Err(e) => {
                    println!("✗ Error receiving tool calls: {}", e);
                }
            }
        }
        _ = tokio::time::sleep(tokio::time::Duration::from_secs(10)) => {
            println!("✗ Timeout waiting for tool calls");
        }
    }

    println!("\nClosing connection...");
    client.close().await?;
    println!("✓ Test complete!");

    Ok(())
}
