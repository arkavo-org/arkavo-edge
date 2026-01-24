# arkavo-claude-code

Claude Agent SDK integration for Arkavo Edge, providing secure and policy-controlled coding capabilities.

## Features

- **Claude Agent SDK Integration**: Full support for Anthropic's advanced coding agent capabilities.
- **Secure Node.js Bridge**: Robust communication layer between Rust and the Node.js SDK environment.
- **Workspace Sandboxing**: Strict file operation confinement to configured workspace roots with path-traversal protection.
- **Policy-Controlled Execution**: Granular permission management for sensitive operations (read, write, exec).
- **Event Mapping**: Real-time translation of SDK events to the Arkavo system-wide event bus.
- **Budget & Token Management**: Integrated tracking of token consumption and budget enforcement for coding tasks.
- **Audit Logging**: Comprehensive structured logging of all tool invocations for security and compliance.
