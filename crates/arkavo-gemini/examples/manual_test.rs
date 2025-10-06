use arkavo_gemini::{LiveSessionClient, ToolDispatcher, ToolRegistry};
use serde_json::json;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let api_key = env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY environment variable required");
    let model = "models/gemini-2.0-flash-exp";

    println!("Creating Gemini Live API client...");
    let client = LiveSessionClient::new(api_key, model);

    println!("Setting up tool registry...");
    let dispatcher = ToolDispatcher::new(2);
    let mut registry = ToolRegistry::new();

    registry.register(
        "create_stream",
        "Creates a new stream with specified name and openness",
        json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Name of the stream"},
                "openness": {"type": "string", "description": "Openness level: Open or Closed"}
            },
            "required": ["name", "openness"]
        }),
        |args| {
            let name = args["name"].as_str().unwrap_or("unknown");
            let openness = args["openness"].as_str().unwrap_or("unknown");
            println!(
                "✓ Tool executed: create_stream(name={}, openness={})",
                name, openness
            );
            Ok(json!({
                "id": format!("stream-{}", uuid::Uuid::new_v4()),
                "name": name,
                "openness": openness,
                "message": format!("Stream '{}' created successfully as {}", name, openness)
            }))
        },
    );

    registry.register(
        "echo",
        "Echoes back the provided message",
        json!({
            "type": "object",
            "properties": {
                "message": {"type": "string", "description": "Message to echo"}
            },
            "required": ["message"]
        }),
        |args| {
            let message = args["message"].as_str().unwrap_or("");
            println!("✓ Tool executed: echo(message={})", message);
            Ok(json!({"echo": message}))
        },
    );

    registry.build(&dispatcher);
    println!("Registered {} tools", dispatcher.list_tools().len());

    println!("\nConnecting to Gemini Live API...");
    client.connect().await?;
    println!("✓ Connected successfully!");

    println!("\nSending test prompt...");
    client
        .send_prompt("Please create a stream called 'test-stream' that is Open")
        .await?;
    println!("✓ Prompt sent");

    println!("\nWaiting for tool calls from Gemini...");
    tokio::select! {
        result = client.receive_tool_calls() => {
            match result {
                Ok(calls) => {
                    println!("✓ Received {} tool call(s)", calls.len());
                    for call in &calls {
                        println!("  - Function: {}", call.name);
                        println!("    Args: {}", call.args);
                        println!("    ID: {}", call.id);
                    }

                    println!("\nDispatching tools...");
                    let results = dispatcher.dispatch(calls).await;

                    println!("\nSending tool responses...");
                    for (id, result) in results {
                        match result {
                            Ok(value) => {
                                println!("✓ Tool {} succeeded: {}", id, value);
                                client.send_tool_response(&id, value).await?;
                            }
                            Err(e) => {
                                println!("✗ Tool {} failed: {}", id, e);
                                client.send_tool_response(&id, json!({"error": e.to_string()})).await?;
                            }
                        }
                    }
                    println!("\n✓ All tool responses sent");
                }
                Err(e) => {
                    println!("✗ Error receiving tool calls: {}", e);
                }
            }
        }
        _ = tokio::time::sleep(tokio::time::Duration::from_secs(30)) => {
            println!("✗ Timeout waiting for tool calls (30s)");
        }
    }

    println!("\nClosing connection...");
    client.close().await?;
    println!("✓ Test complete!");

    Ok(())
}
