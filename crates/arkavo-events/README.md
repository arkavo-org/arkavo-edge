# arkavo-events

Standardized event model and schema for system-wide agent session tracking and audit logging.

## Features

- **Unified Event Model**: Standardized schema for all agent activities including reasoning, tool calls, and results.
- **Structured Payloads**: Type-safe payload definitions for diverse event types across the Arkavo ecosystem.
- **High-Performance Writer**: Asynchronous, buffered event logging with configurable rotation and retention.
- **Multi-Format Serialization**: Support for both JSON (human-readable) and Bincode (efficient binary) formats.
- **Session Tracking**: Built-in support for session IDs and parent event tracking for complex agent workflows.
- **Audit-Ready Metadata**: Comprehensive event metadata including timestamps, agent IDs, and sequence numbers.