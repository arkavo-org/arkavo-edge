# Arkavo Dataflow Blueprint Examples

This directory contains example blueprints demonstrating various dataflow patterns with LLM integration.

## LLM Integration Examples

### llm-summarizer.json
A simple pipeline that receives data via webhook and summarizes it using a local Ollama instance.

**Key features:**
- Single LLM transform node
- Webhook source for receiving data
- Console output for results

**Usage:**
```bash
arkavo-dataflow run examples/blueprints/llm-summarizer.json
```

### llm-multi-provider.json
Demonstrates using multiple Ollama instances (local and remote) for different tasks.

**Key features:**
- Two LLM providers: local for analysis, remote for translation
- Parallel processing of the same input
- Results merged and saved to file
- Environment variable for remote Ollama URL

**Setup:**
```bash
export REMOTE_OLLAMA_URL=http://10.0.0.101:11434
arkavo-dataflow run examples/blueprints/llm-multi-provider.json
```

### llm-code-reviewer.yaml
A code review pipeline using a code-focused model (devstral).

**Key features:**
- YAML format blueprint
- Code-specific prompt engineering
- Low temperature for consistent reviews
- Metadata enrichment
- Append mode for collecting reviews

### llm-router-pattern.json
Advanced pattern showing LLM-based content routing.

**Key features:**
- Initial classification using lightweight model (qwen3:0.6b)
- Conditional routing based on classification
- Different specialized models for each category
- Demonstrates model selection strategy

## LLM Node Parameters

The `llm_transform` node supports the following parameters:

- `provider`: LLM provider to use (e.g., "local-ollama", "remote-ollama")
- `model`: Specific model to use (e.g., "llama3.2:latest", "devstral:latest")
- `task_type`: Optional task type for model preference lookup (e.g., "code_review", "translation")
- `prompt`: Prompt template with {{input}} placeholder
- `temperature`: Controls randomness (0.0-1.0)
- `max_tokens`: Maximum response length
- `timeout_secs`: Request timeout in seconds (default: 30)
- `stream`: Enable streaming responses (default: false)
- `auth_ref`: Authentication reference for API keys

## Dynamic Configuration

The LLM transform node supports AI-driven configuration through memory storage:

1. **Provider Configuration**: The agent can store multiple Ollama provider configurations in memory
2. **Task-Based Model Selection**: Models can be associated with specific task types
3. **Runtime Discovery**: The agent can discover and configure new Ollama instances at runtime

Example memory-stored configuration:
```json
{
  "providers": [
    {
      "name": "local-ollama",
      "base_url": "http://localhost:11434",
      "description": "Local development instance"
    },
    {
      "name": "edge-box",
      "base_url": "http://10.0.0.101:11434",
      "default_model": "devstral:latest",
      "description": "Edge box with specialized models"
    }
  ],
  "model_preferences": {
    "code_review": "devstral:latest",
    "translation": "qwen3:latest",
    "summarization": "llama3.2:latest"
  }
}
```

## Running Examples

1. Ensure Ollama is running locally:
   ```bash
   ollama serve
   ```

2. Pull required models:
   ```bash
   ollama pull llama3.2:latest
   ollama pull devstral:latest
   ollama pull qwen3:0.6b
   ```

3. For multi-provider setup, configure remote Ollama:
   ```bash
   # On remote machine (e.g., 10.0.0.101)
   OLLAMA_HOST=0.0.0.0:11434 ollama serve
   ```

4. Run a blueprint:
   ```bash
   arkavo-dataflow run examples/blueprints/llm-summarizer.json
   ```

## Environment Variables

- `REMOTE_OLLAMA_URL`: URL for remote Ollama instance
- `REMOTE_OLLAMA_API_KEY`: Optional API key for remote instance
- `OLLAMA_BASE_URL`: Override default local Ollama URL
- `OLLAMA_MODEL`: Default model if not specified in blueprint