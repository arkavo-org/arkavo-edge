# arkavo-llm

Unified LLM orchestration and integration layer for Arkavo Edge agents.

## Features

- **Multi-Provider Orchestration**: Single interface for Gemini, Claude, Kimi, DeepSeek, and local llama.cpp models.
- **Standardized Messaging**: Unified message and role definitions across all supported LLM providers.
- **Real-Time Streaming**: Delta-based streaming architecture for low-latency interactive responses.
- **Integrated Tool Execution**: Built-in tool parsing, validation, and routing for MCP-compliant capabilities.
- **Multimodal Support**: Native handling of image and text modalities for advanced vision tasks.
- **Stream Adapters**: Intelligent adapters for converting provider-specific streams into a unified internal format.
