# arkavo-qwen

Qwen-3 (Instruct + Vision) provider for Arkavo Edge via DashScope's OpenAI-compatible API.

## Features

- **Text/Instruct Models**: Full Qwen-3 series support (qwen3-235b-a22b, qwen3-32b, qwen3-14b, etc.)
- **Vision Models**: Qwen-VL and Qwen3-VL support for image understanding
- **Streaming**: Server-sent events (SSE) streaming for real-time responses
- **Function Calling**: Tool/function calling support for agentic workflows
- **Regional Endpoints**: Support for both International and China regions
- **OpenAI-Compatible**: Uses DashScope's OpenAI-compatible API

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
arkavo-qwen = { path = "../arkavo-qwen" }
```

Or with `arkavo-llm`:

```toml
[dependencies]
arkavo-llm = { path = "../arkavo-llm", features = ["qwen"] }
```

## Quick Start

### Basic Chat Completion

```rust
use arkavo_qwen::{QwenProvider, QwenConfig, QwenRegion, Message, MessageRole, Provider};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = QwenConfig {
        api_key: std::env::var("DASHSCOPE_API_KEY")?,
        region: QwenRegion::International,
        model: "qwen3-32b".to_string(),
        ..Default::default()
    };

    let provider = QwenProvider::new(config)?;

    let messages = vec![
        Message {
            role: MessageRole::System,
            content: "You are a helpful assistant.".to_string(),
            images: None,
        },
        Message {
            role: MessageRole::User,
            content: "What is Rust?".to_string(),
            images: None,
        },
    ];

    let response = provider.complete(messages).await?;
    println!("{}", response);

    Ok(())
}
```

### Vision (Image Understanding)

```rust
use arkavo_qwen::{QwenProvider, QwenConfig, Message, MessageRole, Provider};
use arkavo_qwen::vision::encode_image_file;
use std::path::Path;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = QwenConfig {
        api_key: std::env::var("DASHSCOPE_API_KEY")?,
        model: "qwen-vl-max-latest".to_string(),
        ..Default::default()
    };

    let provider = QwenProvider::new(config)?;

    // Encode image from file
    let image_base64 = encode_image_file(Path::new("photo.jpg"))?;

    let messages = vec![
        Message {
            role: MessageRole::User,
            content: "Describe this image in detail.".to_string(),
            images: Some(vec![image_base64]),
        },
    ];

    let response = provider.complete(messages).await?;
    println!("{}", response);

    Ok(())
}
```

### Streaming Responses

```rust
use arkavo_qwen::{QwenProvider, QwenConfig, Message, MessageRole, Provider};
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = QwenConfig::default();
    let provider = QwenProvider::new(config)?;

    let messages = vec![
        Message {
            role: MessageRole::User,
            content: "Write a short poem about Rust programming.".to_string(),
            images: None,
        },
    ];

    let mut stream = provider.stream(messages).await?;

    while let Some(result) = stream.next().await {
        let response = result?;
        print!("{}", response.content);
        if response.done {
            break;
        }
    }

    Ok(())
}
```

### Environment Configuration

```rust
use arkavo_qwen::{QwenProvider, Provider};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Requires DASHSCOPE_API_KEY environment variable
    // Optional: DASHSCOPE_REGION (intl or cn)
    // Optional: QWEN_MODEL
    let provider = QwenProvider::from_env()?;

    // Use provider...
    Ok(())
}
```

## Supported Models

### Text/Instruct Models

- `qwen3-235b-a22b` - Largest open-source Qwen-3 model
- `qwen3-32b` - Balanced performance and efficiency (default)
- `qwen3-30b-a3b` - MoE variant
- `qwen3-14b`, `qwen3-8b`, `qwen3-4b` - Smaller models
- `qwen3-1.7b`, `qwen3-0.6b` - Tiny models for edge deployment
- `qwen-max`, `qwen-plus`, `qwen-turbo` - Commercial snapshots

### Vision Models

- `qwen-vl-max`, `qwen-vl-max-latest` - Production vision model
- `qwen-vl-plus`, `qwen-vl-plus-latest` - Balanced vision model
- `qwen3-vl-plus` - Latest Qwen-3 vision (supports thinking mode)
- `qwen3-vl-235b-a22b-thinking` - Open-source with thinking
- `qwen3-vl-235b-a22b-instruct` - Open-source instruct

## Regional Endpoints

### International (Default)

```rust
use arkavo_qwen::{QwenConfig, QwenRegion};

let config = QwenConfig {
    region: QwenRegion::International, // Uses https://dashscope-intl.aliyuncs.com
    ..Default::default()
};
```

### China

```rust
let config = QwenConfig {
    region: QwenRegion::China, // Uses https://dashscope.aliyuncs.com
    ..Default::default()
};
```

Or via environment variable:

```bash
export DASHSCOPE_REGION=cn  # or "intl"
```

## Environment Variables

- `DASHSCOPE_API_KEY` - DashScope API key (required)
- `DASHSCOPE_BASE_URL` - Override base URL (optional)
- `DASHSCOPE_REGION` - Region: `intl` or `cn` (optional, default: `intl`)
- `QWEN_MODEL` - Default model name (optional, default: `qwen3-32b`)

## Builder Pattern

```rust
use arkavo_qwen::{QwenConfig, QwenProvider};

let provider = QwenProvider::new(QwenConfig::default())?
    .with_temperature(0.8)
    .with_top_p(0.9)
    .with_max_tokens(2048);
```

## Error Handling

```rust
use arkavo_qwen::{QwenError, Result};

match provider.complete(messages).await {
    Ok(response) => println!("{}", response),
    Err(QwenError::AuthenticationFailed { message }) => {
        eprintln!("Auth error: {}", message);
    }
    Err(QwenError::RateLimitExceeded { message, retry_after }) => {
        eprintln!("Rate limit: {}, retry after: {:?}s", message, retry_after);
    }
    Err(QwenError::ModelNotFound { model }) => {
        eprintln!("Model not found: {}", model);
    }
    Err(e) => eprintln!("Error: {}", e),
}
```

## Vision Helpers

```rust
use arkavo_qwen::vision::{create_image_part, create_image_url_from_file, validate_image_size};
use std::path::Path;

// Encode image from file with auto-detected MIME type
let image_url = create_image_url_from_file(Path::new("photo.jpg"))?;

// Validate image size (max 10MB)
validate_image_size(&base64_data, 10)?;

// Create vision content part
let part = create_image_part(&base64_data);
```

## Testing

```bash
# Run unit tests
cargo test

# Run integration tests (requires mock server)
cargo test --test integration_test

# Run all tests with output
cargo test -- --nocapture
```

## Examples

See `tests/integration_test.rs` for complete examples including:

- Basic chat completion
- Vision with images
- Streaming responses
- Error handling
- Region configuration

## License

Apache-2.0

## Links

- [DashScope Documentation](https://help.aliyun.com/zh/dashscope/)
- [Qwen-3 Release](https://qwenlm.github.io/blog/qwen3/)
- [Qwen-VL Models](https://help.aliyun.com/zh/dashscope/developer-reference/qwen-vl-api)
