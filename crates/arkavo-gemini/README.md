# arkavo-gemini

Gemini API integration with streaming support and advanced function calling for Arkavo Edge.

## Features

- **Streaming REST Client**: Sub-second time-to-first-token (TTFT) with incremental text and tool call streaming.
- **Advanced Function Calling**: Full integration with Gemini's tool use capabilities including schema validation.
- **Concurrent Tool Dispatcher**: High-performance execution of multiple tool calls with semaphore-based rate limiting.
- **Live API Integration**: WebSocket-based support for real-time multimodal (audio/video) conversations.
- **Idempotent Execution**: Built-in request deduplication to ensure reliable tool execution in unstable network conditions.
- **Flexible Modality Support**: Support for text, audio, and video inputs across supported Gemini models.