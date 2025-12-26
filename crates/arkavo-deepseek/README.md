# arkavo-deepseek

DeepSeek LLM integration with dual-API compatibility and advanced function calling for Arkavo Edge.

## Features

- **Dual API Support**: Full compatibility with both OpenAI-style and Anthropic-style message formats.
- **Advanced Function Calling**: Native support for up to 128 concurrent tools/functions per request.
- **Strict Validation Mode**: JSON Schema enforcement for function parameters to ensure response reliability.
- **Real-Time Streaming**: High-performance streaming support via Server-Sent Events (SSE).
- **Specialized Model Support**: Optimized integration for `deepseek-chat` and `deepseek-reasoner` models.
- **Anthropic Compatibility Bridge**: Automatic conversion between Anthropic message formats and DeepSeek APIs.
