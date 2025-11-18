//! Streaming tool test example
//!
//! This example uses #[tokio::main] which internally uses Runtime::block_on.
#![allow(clippy::disallowed_methods)]

use arkavo_gemini::{
    FunctionDeclaration, GeminiSseStream, RestClient, ToolDispatcher, ToolRegistry,
};
use serde_json::json;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let api_key = env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY required");
    let model =
        env::var("GEMINI_MODEL").unwrap_or_else(|_| "models/gemini-3-pro-preview".to_string());

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
            println!(
                "✓ Tool executed: create_stream(name={name}, openness={openness})"
            );
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

    let tools: Vec<FunctionDeclaration> = tool_schemas
        .iter()
        .map(|t| FunctionDeclaration {
            name: t["name"].as_str().unwrap().to_string(),
            description: t["description"].as_str().unwrap().to_string(),
            parameters: t["parameters"].clone(),
        })
        .collect();

    println!("✓ Registered {} tools\n", tools.len());
    println!("Creating REST client...");
    println!("Using model: {}\n", model);

    let client = RestClient::new(api_key, &model);

    println!("Starting streaming request...\n");
    let start_time = std::time::Instant::now();

    let mut stream: GeminiSseStream = client
        .stream_generate_content(
            "Please create a new stream called 'release-canary' and make it Open",
            Some(tools),
        )
        .await?;

    println!("Stream started! Receiving responses...\n");

    let mut first_token_time = None;
    let mut response_count = 0;

    while let Some(result) = stream.next().await {
        match result {
            Ok(response) => {
                response_count += 1;

                if first_token_time.is_none() {
                    first_token_time = Some(start_time.elapsed());
                    println!("⏱  Time to first token: {:?}\n", first_token_time.unwrap());
                }

                if let Some(text) = &response.text {
                    print!("{text}");
                }

                if !response.function_calls.is_empty() {
                    println!(
                        "\n\n✓ Received {} tool call(s)!\n",
                        response.function_calls.len()
                    );
                    for call in &response.function_calls {
                        println!("  Function: {}", call.name);
                        println!("  Args: {}", serde_json::to_string_pretty(&call.args)?);
                        println!("  ID: {}\n", call.id);
                    }

                    let tool_start = std::time::Instant::now();
                    println!("Dispatching tools...");
                    let results = dispatcher.dispatch(response.function_calls.clone()).await;
                    let tool_duration = tool_start.elapsed();

                    println!("✓ Tool execution completed in {tool_duration:?}\n");

                    for (id, result) in results {
                        match result {
                            Ok(value) => {
                                println!("✓ Tool {id} succeeded");
                                println!("  Result: {}\n", serde_json::to_string_pretty(&value)?);
                            }
                            Err(e) => {
                                println!("✗ Tool {id} failed: {e}\n");
                            }
                        }
                    }
                }

                if response.done {
                    println!("\n\n✓ Stream complete!");
                    break;
                }
            }
            Err(e) => {
                println!("\n✗ Stream error: {e}");
                break;
            }
        }
    }

    let total_time = start_time.elapsed();

    println!("\n─────────────────────────────────────");
    println!("Performance Metrics:");
    println!("─────────────────────────────────────");
    println!("Total responses: {response_count}");
    if let Some(ttft) = first_token_time {
        println!("Time to first token (TTFT): {ttft:?}");
    }
    println!("Total duration: {total_time:?}");
    println!("─────────────────────────────────────");

    Ok(())
}
