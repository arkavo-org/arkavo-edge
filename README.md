# Arkavo Edge

A developer-centric agentic CLI tool for AI-agent development and framework maintenance, focusing on secure, cost-efficient runtime for multi-file code transformations.

## Features

- Local LLM inference with Qwen3-0.6B (privacy-first, no API required)

### Planned

- Conversational agent with repository context
- Change planning and execution with automatic commit generation
- Test integration with the agent feedback loop
- GPU-accelerated terminal UI
- Repository mapping and file tracking
- Encrypted storage with Edge Vault

## Usage

```bash
# Start conversational agent
arkavo chat

# Generate a change plan before edits
arkavo plan

# Execute plan and commit changes
arkavo apply

# Run tests with streaming failure feedback
arkavo test

# Import/export notes to Edge Vault
arkavo vault
```

## Development

```bash
# Build the project
cargo build

# Run the project
cargo run

# Run tests
cargo test

# Code quality
cargo clippy -D warnings

# Format code
cargo fmt
```

## Platforms

- macOS (arm64)
- Linux (x64/aarch64)
- Windows (future support)

## License

Arkavo Edge is licensed under the [Apache License, Version 2.0](http://www.apache.org/licenses/LICENSE-2.0).

```
Copyright 2025 Arkavo Edge Contributors

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```