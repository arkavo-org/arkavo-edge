# arkavo-agui

AG-UI (Agentic GUI) protocol implementation and web gateway for Arkavo Edge.

## Features

- **AI-Driven UI Generation**: Prompt-to-UI system that generates production-ready web components.
- **Real-time Streaming**: Streams HTML, CSS, and JavaScript directly from LLM providers.
- **WebSocket Protocol**: Real-time bidirectional communication for live UI updates and interactions.
- **Status Monitoring**: Integrated monitoring for system health, MCP tools, and LLM connections.
- **AG-UI Protocol**: Full implementation of the Agentic GUI event protocol for agent-to-user interaction.
- **MCP Tool Integration**: Designed to leverage MCP tools for data-driven UI generation and refinement.

## Configuration

- `ARKAVO_AGUI_BIND`: IP address the gateway listens on. Defaults to `127.0.0.1` (loopback only). The gateway has no authentication, so set this explicitly (e.g. `0.0.0.0`) only when you intentionally want remote browser access, such as developing from another machine. Invalid values fall back to loopback.