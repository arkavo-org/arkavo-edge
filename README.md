# Arkavo Edge

A developer-centric agentic CLI tool for AI-agent development and framework maintenance, focusing on secure, cost-efficient runtime for multi-file code transformations.

## Features

- Conversational agent with repository context
- Change planning and execution with automatic commit generation
- Test integration with the agent feedback loop
- GPU-accelerated terminal UI
- Repository mapping and file tracking
- Encrypted storage with Edge Vault
- **AI-powered test harness with MCP server support**

## Test Harness with MCP Server

Arkavo Edge includes an AI-driven test harness that can be used as an MCP (Model Context Protocol) server, making it ideal for use with Claude Code and other AI development tools.

### Key Capabilities

- **Gherkin/BDD Support**: Parse and execute `.feature` files with natural language test specifications
- **MCP Server Integration**: Expose test tools via MCP protocol for AI-driven testing
- **State Management**: Query, mutate, and snapshot application state during tests
- **Business-Readable Reports**: Generate reports in Markdown, HTML, JSON, and Slack formats
- **Mobile Testing Support**: iOS bridge with FFI for native app testing

### Using with Claude Code

To use the Arkavo test harness as an MCP server in Claude Code:

1. **Build the project first**:
   ```bash
   cargo build --release
   ```

2. **Configure MCP in Claude Code settings**:
   ```json
   {
     "mcpServers": {
       "arkavo": {
         "command": "/path/to/arkavo-edge/target/release/arkavo",
         "args": ["serve"],
         "cwd": "/path/to/your/project"
       }
     }
   }
   ```

3. **Available MCP Tools**:
   - `query_state`: Query application state (e.g., `{"entity": "user", "filter": {"field": "balance"}}`)
   - `mutate_state`: Modify application state (e.g., `{"entity": "user", "action": "update", "data": {...}}`)
   - `snapshot`: Create/restore state snapshots (e.g., `{"action": "create", "name": "checkpoint1"}`)
   - `run_test`: Execute test scenarios (e.g., `{"test_name": "login_test", "timeout": 30}`)

3. **Example Gherkin Test**:
   ```gherkin
   Feature: User Authentication
     Scenario: Successful login
       Given a user with valid credentials
       When the user attempts to login
       Then the login should succeed
       And the user should see the dashboard
   ```

### Running Tests

```bash
# Run all tests in current project
arkavo test

# Run BDD/Gherkin tests
arkavo test --bdd

# Run with specific feature file
arkavo test --feature tests/auth.feature

# Generate test plan from natural language
arkavo test --plan "Test user login with 2FA enabled"
```

### Example Integration

See `crates/arkavo-test/examples/` for complete examples:
- `test_mcp_server.rs`: Demonstrates MCP server functionality
- `simple_test_demo.rs`: Shows Gherkin test execution
- `banking_app.feature`: Example BDD test specification

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

# Run as MCP server for AI tools integration
arkavo serve
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