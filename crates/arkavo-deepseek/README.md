# arkavo-deepseek

DeepSeek provider with Anthropic-compatible API and Function Calling support for Arkavo Edge.

## Features

- **Dual API Compatibility**: Supports both OpenAI-style and Anthropic-style APIs
- **Function Calling**: Full support for up to 128 tools/functions per request
- **Strict Mode (Beta)**: JSON Schema validation for function parameters
- **Streaming**: Server-Sent Events (SSE) streaming support
- **Model Support**: Works with `deepseek-chat` and `deepseek-reasoner` models
- **Error Handling**: Comprehensive error mapping and retry logic
- **Anthropic Compatibility**: Handles Anthropic message format conversion

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
arkavo-deepseek = { path = "../arkavo-deepseek" }
```

## Usage

### Basic Completion

```rust
use arkavo_deepseek::{DeepSeekProvider, Message, MessageRole, Provider};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create provider from environment variables (DEEPSEEK_API_KEY)
    let provider = DeepSeekProvider::from_env()?;
    
    let messages = vec![
        Message {
            role: MessageRole::System,
            content: "You are a helpful assistant.".to_string(),
            images: None,
        },
        Message {
            role: MessageRole::User,
            content: "What is 2+2?".to_string(),
            images: None,
        },
    ];
    
    let response = provider.complete(messages).await?;
    println!("Response: {}", response);
    
    Ok(())
}
```

### Streaming Response

```rust
use arkavo_deepseek::{DeepSeekProvider, Message, MessageRole, Provider};
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = DeepSeekProvider::from_env()?;
    
    let messages = vec![
        Message {
            role: MessageRole::User,
            content: "Tell me a story".to_string(),
            images: None,
        },
    ];
    
    let mut stream = provider.stream(messages).await?;
    
    while let Some(result) = stream.next().await {
        match result {
            Ok(response) => {
                if let Some(content) = response.content {
                    print!("{}", content);
                }
                if response.done {
                    println!("\n[Stream complete]");
                    break;
                }
            }
            Err(e) => eprintln!("Stream error: {}", e),
        }
    }
    
    Ok(())
}
```

### Function Calling

```rust
use arkavo_deepseek::{
    ChatMessage, DeepSeekClient, DeepSeekConfig, MessageContent, Role, 
    Tool, ToolChoice, ToolFunction,
};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = DeepSeekConfig {
        api_key: std::env::var("DEEPSEEK_API_KEY")?,
        model: "deepseek-chat".to_string(),
        ..Default::default()
    };
    
    let client = DeepSeekClient::new(config)?;
    
    // Define a tool
    let weather_tool = Tool {
        tool_type: "function".to_string(),
        function: ToolFunction {
            name: "get_weather".to_string(),
            description: "Get weather for a location".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "location": {
                        "type": "string",
                        "description": "City name"
                    }
                },
                "required": ["location"],
                "additionalProperties": false
            }),
        },
        strict: None,
    };
    
    let messages = vec![
        ChatMessage {
            role: Role::User,
            content: MessageContent::Text {
                content: "What's the weather in Paris?".to_string(),
            },
            name: None,
            tool_calls: None,
            tool_call_id: None,
        },
    ];
    
    let response = client
        .complete(messages, Some(vec![weather_tool]), Some(ToolChoice::Auto), None)
        .await?;
    
    // Check for tool calls in response
    if let Some(tool_calls) = &response.choices[0].message.tool_calls {
        for tool_call in tool_calls {
            println!("Tool: {}", tool_call.function.name);
            println!("Arguments: {}", tool_call.function.arguments);
            
            // Validate and parse arguments
            let args_json: serde_json::Value = 
                serde_json::from_str(&tool_call.function.arguments)?;
            println!("Location: {}", args_json["location"]);
        }
    }
    
    Ok(())
}
```

### Strict Mode (Beta)

```rust
use arkavo_deepseek::{DeepSeekClient, DeepSeekConfig};

let config = DeepSeekConfig {
    api_key: std::env::var("DEEPSEEK_API_KEY")?,
    use_strict_mode: true,  // Enable strict mode
    base_url: "https://api.deepseek.com/beta".to_string(),
    ..Default::default()
};

let client = DeepSeekClient::new(config)?;

// When using tools with strict mode, schemas are validated:
// - All properties must be in required array
// - additionalProperties: false is mandatory
// - Certain constraints are enforced
```

### Anthropic Compatibility

The crate can handle Anthropic-style message formats:

```rust
use arkavo_deepseek::anthropic_compat::{convert_anthropic_request};
use arkavo_deepseek::{AnthropicMessage, AnthropicMessageRequest};

// Create an Anthropic-style request
let request = AnthropicMessageRequest {
    model: "deepseek-chat".to_string(),
    messages: vec![/* Anthropic messages */],
    system: Some("System prompt".to_string()),
    max_tokens: Some(1000),
    temperature: Some(0.7),
    // ... other fields
};

// Convert to DeepSeek format
let (messages, tools, tool_choice) = convert_anthropic_request(request)?;
```

## Configuration

### Environment Variables

- `DEEPSEEK_API_KEY` - Your DeepSeek API key (required)
- `DEEPSEEK_BASE_URL` - Override base URL (default: `https://api.deepseek.com`)
- `DEEPSEEK_MODEL` - Default model (default: `deepseek-chat`)
- `DEEPSEEK_STRICT_MODE` - Enable strict mode by default (default: `false`)

### Configuration Options

```rust
use arkavo_deepseek::DeepSeekConfig;
use std::time::Duration;

let config = DeepSeekConfig {
    api_key: "your-api-key".to_string(),
    base_url: "https://api.deepseek.com".to_string(),
    model: "deepseek-chat".to_string(),
    use_strict_mode: false,
    anthropic_compat: false,
    max_tokens: Some(4096),
    temperature: Some(0.7),
    top_p: None,
    timeout: Duration::from_secs(60),
    max_retries: 3,
};
```

## Models

### deepseek-chat
- Supports function calling
- 128K context window
- Optimized for general conversation

### deepseek-reasoner
- Enhanced reasoning capabilities
- Does NOT support function calling (falls back to deepseek-chat when tools are present)
- 128K context window
- Includes reasoning_content in responses

## Error Handling

The crate provides comprehensive error types:

```rust
use arkavo_deepseek::DeepSeekError;

match result {
    Err(DeepSeekError::RateLimited { retry_after, .. }) => {
        // Handle rate limiting with optional retry_after duration
    }
    Err(DeepSeekError::AuthenticationFailed { .. }) => {
        // Handle auth errors
    }
    Err(DeepSeekError::SchemaError { .. }) => {
        // Handle strict mode schema validation errors
    }
    Err(DeepSeekError::ToolArgumentsInvalid { .. }) => {
        // Handle invalid tool arguments
    }
    // ... other error types
    _ => {}
}
```

## Testing

Run tests with:

```bash
cargo test -p arkavo-deepseek
```

For live API tests (requires `DEEPSEEK_API_KEY`):

```bash
cargo test -p arkavo-deepseek -- --ignored
```

## License

Apache-2.0

## Contributing

Contributions are welcome! Please ensure all tests pass and follow the project's coding standards.