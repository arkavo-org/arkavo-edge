# Multi-Ollama Server Configuration

Arkavo Edge supports connecting to multiple Ollama servers simultaneously, following the AI-driven configuration principle from CLAUDE.md.

## How It Works

1. **Server Discovery**: The terminal UI automatically discovers Ollama servers from the memory storage
2. **Interactive Configuration**: Use `arkavo chat` to configure additional Ollama servers
3. **Model Prefixes**: Models are displayed with server prefixes (e.g., `localhost/llama3`, `server1/devstral`)

## Adding Ollama Servers

When you run `arkavo chat`, it will:
1. Check for existing Ollama server configurations in memory storage
2. Test connectivity to saved servers
3. Prompt you to add new servers if needed

The configurations are stored in the memory crate with:
- Content: The server URL (e.g., `http://10.0.0.101:11434`)
- Category: `config`
- Metadata: Server details and capabilities

## Example Workflow

```bash
# First run - configure primary server
$ cargo run -p arkavo chat
✓ Connected to saved Ollama server at http://localhost:11434
# If not found, it will prompt for server URL

# The configuration is saved to memory storage
# Next time you run arkavo-terminal, it will discover all saved servers

# Run terminal UI
$ cargo run -p arkavo-terminal
✓ Connected to localhost Ollama server
✓ Connected to server1 Ollama server: http://10.0.0.101:11434

# Models appear with server prefixes
# Use Tab to cycle through: localhost/llama3, server1/devstral, etc.
```

## Memory Storage Structure

Ollama server configurations are stored as:
```json
{
  "content": "http://10.0.0.101:11434",
  "category": "config",
  "metadata": {
    "type": "ollama_server",
    "name": "Remote Ollama",
    "discovered_at": "2024-01-01T00:00:00Z"
  }
}
```

## Benefits

- **No Environment Variables**: Follows the AI-driven configuration principle
- **Persistent Storage**: Configurations are saved in the memory crate
- **Dynamic Discovery**: Servers are discovered at runtime
- **Multiple Servers**: Connect to any number of Ollama instances
- **Unified Storage**: All configurations use the same memory system used for embeddings, git info, etc.