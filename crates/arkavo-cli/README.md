# arkavo-cli

Core CLI implementation for the Arkavo agentic tool, providing the main entry point and agent execution loop.

## Features

- **Progressive Tool Disclosure**: Efficiently manages LLM token usage by only exposing relevant tools based on query context.
- **Iterative Task Execution**: Multi-step agent loop for planning, tool execution, and response refinement.
- **Quality Gate Integration**: Automated validation of agent responses and tool usage via the Arkavo router.
- **Interactive Chat & Task Modes**: Optimized interfaces for both conversational interactions and complex software engineering tasks.
- **Unified Command Parser**: Centralized management for LLM orchestration, model discovery, and agent configuration.
- **Human-Readable Feedback**: Concise status reporting and transparent tool execution logs for better developer experience.
- **Cross-Model Support**: Seamless switching and orchestration between Gemini, Claude, and local LLMs.