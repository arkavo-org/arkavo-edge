use arkavo_gemini::{FunctionDeclaration, RestClient, ToolDispatcher, ToolRegistry};
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

    let tool_schemas: Vec<FunctionDeclaration> = dispatcher
        .list_tools()
        .iter()
        .map(|t| FunctionDeclaration {
            name: t["name"].as_str().unwrap_or("unknown").to_string(),
            description: t["description"].as_str().unwrap_or("").to_string(),
            parameters: t["parameters"].clone(),
        })
        .collect();

    println!("\nCreating REST API client...");
    let client = RestClient::new(api_key, model);

    println!("Sending request with tool declarations...");
    let (text, calls) = client
        .generate_content(
            "Please create a new stream called 'release-canary' and make it Open",
            Some(tool_schemas),
        )
        .await?;

    if let Some(response_text) = text {
        println!("\n📝 Model response: {}", response_text);
    }

    if !calls.is_empty() {
        println!("\n🔧 Received {} tool call(s)!", calls.len());
        for call in &calls {
            println!("\n  Function: {}", call.name);
            println!("  Args: {}", serde_json::to_string_pretty(&call.args)?);
            println!("  ID: {}", call.id);
        }

        println!("\nDispatching tools...");
        let results = dispatcher.dispatch(calls).await;

        println!("\nTool results:");
        for (id, result) in results {
            match result {
                Ok(value) => {
                    println!("✓ Tool {} succeeded", id);
                    println!("  Result: {}", serde_json::to_string_pretty(&value)?);
                }
                Err(e) => {
                    println!("✗ Tool {} failed: {}", id, e);
                }
            }
        }
    } else {
        println!("\n⚠️  No tool calls received");
    }

    Ok(())
}
