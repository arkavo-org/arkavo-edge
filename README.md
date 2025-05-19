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

## LLM Integration

Arkavo Edge uses the Qwen3-0.6B model for local LLM inference, ensuring privacy with no external API dependencies.

### Downloading and Setting Up Qwen3-0.6B

#### Hugging Face Authentication

To download the model from Hugging Face:

1. Visit [Hugging Face Token Settings](https://huggingface.co/settings/tokens)
2. Create a _token with "Read" permission
3. Enter the _token in the CLI `huggingface-cli`

#### Download the Model

Create models directory
```bash
mkdir -p crates/arkavo-llm/models
```

Download the model directly to the target location

Base model:
```bash
huggingface-cli download Qwen/Qwen3-0.6B-Base --local-dir crates/arkavo-llm/models
```

garbage output:
```bash
huggingface-cli download suayptalha/Qwen3-0.6B-Code-Expert --local-dir crates/arkavo-llm/models
```

This single command downloads all required model files directly where Arkavo expects them. The download takes approximately 1-2 minutes depending on your connection speed.

#### Using the Model

```bash
# Start interactive chat
arkavo chat

# Process a single prompt
arkavo chat --prompt "Hello Cyberspace"
```

### Embedding the Model in the Binary

For completely offline distribution, Arkavo Edge supports embedding the model directly into the binary. This is particularly useful for deployments where downloading the model is not possible or desirable.

#### Key Benefits

- **100% Offline Operation**: No internet connection required after deployment
- **Self-contained Binary**: Single-file distribution simplifies deployment
- **Secure Runtime**: No downloads or external dependencies
- **Portability**: Works across all supported platforms

#### Size Considerations

- The final binary will be significantly larger (1.5-2GB)
- Building may take longer due to embedding the large model files
- Runtime memory usage remains the same as with external model files

### Technical Implementation

Arkavo uses the Candle framework (by Hugging Face) for efficient Rust-based ML inference. Key integration points:

- **CPU and GPU Support**: Both CPU and GPU inference are supported, with automatic fallback to CPU if GPU is unavailable
- **Quantization**: The model uses efficient quantization techniques for optimal performance even on modest hardware
- **Memory Efficiency**: Designed to work within reasonable memory constraints while maintaining quality outputs
- **Privacy**: All processing occurs locally on your device with no data sent to external services
- **Secure Cleanup**: Temporary files are securely deleted after use in high-security environments

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