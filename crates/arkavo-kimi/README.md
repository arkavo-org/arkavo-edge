# arkavo-kimi

Native Kimi (Moonshot) API integration for Arkavo Edge.

## Features

- Direct Kimi API implementation without OpenAI abstraction layers
- Full support for function/tool calling
- Built-in exponential backoff retry logic
- Proper SSE streaming support
- Rate limit handling with Retry-After header support
- Compile-time model validation

## Usage

### Basic Chat Completion

```rust
use arkavo_kimi::{KimiConfig, KimiProvider};
use arkavo_llm::{Provider, Message};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create provider from environment variables
    let provider = KimiProvider::from_env()?;
    
    // Or create with explicit config
    let config = KimiConfig {
        api_key: "your-api-key".to_string(),
        model: arkavo_kimi::Model::MoonshotV1_8k,
        ..Default::default()
    };
    let provider = KimiProvider::new(config)?;
    
    // Create messages
    let messages = vec![
        Message::system("You are a helpful assistant"),
        Message::user("What is the capital of France?"),
    ];
    
    // Get completion
    let response = provider.complete(messages).await?;
    println!("Response: {}", response);
    
    Ok(())
}
```

### Streaming Response

```rust
use futures::StreamExt;

let mut stream = provider.stream(messages).await?;

while let Some(chunk) = stream.next().await {
    match chunk {
        Ok(response) => {
            print!("{}", response.content);
            if response.done {
                println!("\n[Stream complete]");
            }
        }
        Err(e) => eprintln!("Stream error: {}", e),
    }
}
```

### Using Tools/Functions

```rust
use arkavo_kimi::{KimiClient, KimiConfig, Tool, ToolFunction, ToolChoice};
use serde_json::json;

let client = KimiClient::new(KimiConfig::from_env()?)?;

let tools = vec![Tool {
    tool_type: "function".to_string(),
    function: ToolFunction {
        name: "get_weather".to_string(),
        description: "Get the current weather for a location".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "location": {
                    "type": "string",
                    "description": "The city and country"
                }
            },
            "required": ["location"]
        }),
    },
}];

let response = client.create_chat_completion(
    messages,
    Some(tools),
    Some(ToolChoice::Auto),
    Some(0.7),  // temperature
    None,       // top_p
).await?;
```

## Environment Variables

- `MOONSHOT_API_KEY` - Your Kimi API key (required)
- `KIMI_API_BASE` - API base URL (optional, defaults to https://api.moonshot.ai/v1)
- `KIMI_MODEL` - Default model to use (optional, defaults to moonshot-v1-8k)

## Models

The crate supports all Kimi models:

- `moonshot-v1-8k` - 8K context window
- `moonshot-v1-32k` - 32K context window
- `moonshot-v1-128k` - 128K context window