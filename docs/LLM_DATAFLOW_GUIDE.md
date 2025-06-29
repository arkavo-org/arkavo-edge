# LLM Dataflow Integration Guide for AI Agents

This guide is specifically for AI agents to understand and use the LLM dataflow capabilities.

## Available MCP Tools

The following MCP tools are available for LLM dataflow operations:

### 1. discover_llm_providers
Discovers available Ollama instances on the network.

**Usage:**
```json
{
  "tool": "discover_llm_providers",
  "params": {}
}
```

**Returns:**
- List of discovered providers with names and URLs
- Capability information about each provider
- Example usage patterns

### 2. configure_llm_providers
Configures multiple LLM providers for use in pipelines.

**Usage:**
```json
{
  "tool": "configure_llm_providers",
  "params": {
    "providers": [
      {"name": "local-ollama", "url": "http://localhost:11434"},
      {"name": "edge-box", "url": "http://10.0.0.101:11434"}
    ]
  }
}
```

### 3. generate_llm_blueprint
Generates a complete dataflow blueprint for a given task.

**Usage:**
```json
{
  "tool": "generate_llm_blueprint",
  "params": {
    "task": "Review code for security vulnerabilities",
    "pipeline_type": "simple"  // options: simple, routing, parallel
  }
}
```

### 4. set_model_preference
Sets preferred models for specific task types.

**Usage:**
```json
{
  "tool": "set_model_preference",
  "params": {
    "task_type": "code_review",
    "model": "devstral:latest"
  }
}
```

## Task Types and Recommended Models

| Task Type | Description | Recommended Models |
|-----------|-------------|-------------------|
| code_review | Review code for quality and issues | devstral:latest, codellama:latest |
| summarization | Create concise summaries | llama3.2:latest, mistral:latest |
| translation | Translate between languages | qwen3:latest, llama3.2:latest |
| classification | Categorize content | qwen3:0.6b, phi3:mini |

## Blueprint Node Configuration

LLM transform nodes support these parameters:

```json
{
  "type": "llm_transform",
  "provider": "local-ollama",      // Provider name
  "model": "llama3.2:latest",      // Optional: specific model
  "task_type": "summarization",    // Optional: for model preference lookup
  "prompt": "Summarize: {{input}}", // Prompt template
  "temperature": 0.5,              // 0.0-1.0
  "max_tokens": 500,               // Response length limit
  "timeout_secs": 30,              // Request timeout
  "stream": false                  // Enable streaming
}
```

## Discovery Process

1. **Initial Discovery**: Use `discover_llm_providers` to find available Ollama instances
2. **Configuration**: Use `configure_llm_providers` to set up discovered endpoints
3. **Task Analysis**: Determine the task type from user requirements
4. **Blueprint Generation**: Use `generate_llm_blueprint` to create appropriate pipeline
5. **Model Preferences**: Use `set_model_preference` for task-specific models

## Memory Storage

All configurations are automatically stored in the arkavo memory system:
- Provider configurations persist across sessions
- Model preferences are remembered per task type
- No manual configuration files needed

## Example Workflow

```python
# 1. Discover available providers
providers = mcp.call("discover_llm_providers")

# 2. Configure discovered providers
mcp.call("configure_llm_providers", {
    "providers": providers["discovered_providers"]
})

# 3. Generate blueprint for user's task
blueprint = mcp.call("generate_llm_blueprint", {
    "task": "Analyze customer feedback sentiment",
    "pipeline_type": "simple"
})

# 4. Set model preference if needed
mcp.call("set_model_preference", {
    "task_type": "sentiment_analysis",
    "model": "llama3.2:latest"
})
```

## Best Practices

1. **Provider Selection**:
   - Use local-ollama for development and quick tasks
   - Use remote providers for specialized models or production

2. **Model Selection**:
   - Small models (qwen3:0.6b) for simple classification
   - Medium models (llama3.2) for general tasks
   - Specialized models (devstral) for code-related tasks

3. **Temperature Settings**:
   - 0.1-0.3 for deterministic tasks (classification, routing)
   - 0.5-0.7 for balanced creativity (summarization)
   - 0.8-1.0 for creative tasks (content generation)

4. **Pipeline Types**:
   - Simple: Direct input → LLM → output
   - Routing: Use LLM to classify and route to different processors
   - Parallel: Process with multiple models simultaneously