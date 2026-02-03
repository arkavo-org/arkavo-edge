# arkavo-kimi

Native Kimi (Moonshot AI) API client for Rust.

Provides direct access to Kimi API features without OpenAI abstraction layers, supporting both legacy V1 models and the latest K2.5 series.

## Features

- **Native API**: Direct Kimi API integration (not OpenAI-compatible layer)
- **K2.5 Series Support**: Full support for Kimi K2.5 models with 256K context window
- **Thinking Mode**: Configurable thinking/reasoning mode for K2.5 series
- **Streaming**: Real-time streaming responses with SSE
- **Tool Calling**: Function calling support
- **Retry Logic**: Exponential backoff with configurable retries
- **Type Safety**: Strongly typed request/response models

## Supported Models

### K2.5 Series (Recommended)
- `kimi-k2.5` - General purpose model with 256K context
- `kimi-k2-0905-preview` - Preview version
- `kimi-k2-turbo-preview` - Faster responses
- `kimi-k2-thinking` - Enhanced reasoning capabilities
- `kimi-k2-thinking-turbo` - Fast reasoning

All K2.5 models support:
- 256K token context window
- Thinking mode (can be enabled/disabled)
- Multimodal inputs (vision)
- Tool calling

### Legacy V1 Series
- `moonshot-v1-8k` - 8K context
- `moonshot-v1-32k` - 32K context
- `moonshot-v1-128k` - 128K context

## API Endpoints

Moonshot AI provides two API endpoints:

### OpenAI-Compatible (this crate)
- URL: `https://api.moonshot.ai/v1`
- Used by `arkavo-kimi` crate

### Anthropic-Compatible
- URL: `https://api.moonshot.ai/anthropic`
- Can be used with Anthropic SDK by setting `ANTHROPIC_BASE_URL`
- Temperature mapping: `real_temp = request_temp * 0.6`

## Usage

```rust
use arkavo_kimi::{KimiClient, KimiConfig, Model, ThinkingConfig};

// Create client with default config (uses K2.5)
let config = KimiConfig::from_env()?;
let client = KimiClient::new(config)?;

// Or configure manually
let config = KimiConfig {
    api_key: "your-api-key".to_string(),
    model: Model::KimiK2_5,
    ..Default::default()
};
```

## Environment Variables

- `MOONSHOT_API_KEY` - Your Moonshot AI API key (required)
- `KIMI_MODEL` - Model to use (default: `kimi-k2.5`)
- `KIMI_API_BASE` - API base URL (default: `https://api.moonshot.ai/v1`)
- `KIMI_USER_AGENT` - User-Agent identifier (default: `arkavo-edge/0.58`)

## Thinking Mode (K2.5 Series)

K2.5 models support a thinking mode that enables step-by-step reasoning:

```rust
use arkavo_kimi::ThinkingConfig;

// Enable thinking mode
let provider = KimiProvider::new(config)?
    .with_thinking(true);

// Or disable for faster responses
let provider = KimiProvider::new(config)?
    .with_thinking(false);
```

## License

MIT OR Apache-2.0
