# Multi-Server Blueprint Example

This example shows how to use models from different Ollama servers in a dataflow blueprint.

## Prerequisites

1. Configure multiple Ollama servers using `arkavo chat`
2. Verify servers are saved in memory storage
3. Models will be available as `server1/model`, `server2/model`, etc.

## Example: Multi-Server Code Review Pipeline

This pipeline uses different models from different servers for specialized tasks:

```yaml
version: 1.0.0
name: multi-server-code-review
description: Code review pipeline using models from multiple Ollama servers

nodes:
  - id: code_input
    kind: source
    params:
      type: webhook_source
      port: 8082
      path: /multi-review
      
  - id: syntax_checker
    kind: transform
    params:
      type: llm_transform
      # Uses model from localhost
      model: localhost/llama3:latest
      prompt: |
        Check the following code for syntax errors and basic issues:
        {{input}}
      temperature: 0.1
      max_tokens: 300
      
  - id: security_reviewer
    kind: transform
    params:
      type: llm_transform
      # Uses specialized model from server1
      model: server1/devstral:latest
      prompt: |
        Review this code for security vulnerabilities:
        - SQL injection
        - XSS risks
        - Authentication issues
        - Data exposure
        
        Code:
        {{input}}
      temperature: 0.2
      max_tokens: 500
      
  - id: performance_analyzer
    kind: transform
    params:
      type: llm_transform
      # Uses model from server2
      model: server2/deepseek-r1:14b
      prompt: |
        Analyze this code for performance issues:
        - Time complexity
        - Memory usage
        - Database queries
        - Caching opportunities
        
        Code:
        {{input}}
      temperature: 0.3
      max_tokens: 400
      
  - id: result_merger
    kind: transform
    params:
      type: json_transform
      spec:
        syntax_check: "syntax_result"
        security_review: "security_result"
        performance_analysis: "performance_result"
        timestamp: "${CURRENT_TIME}"
        
  - id: output
    kind: sink
    params:
      type: file_sink
      path: multi_server_reviews.json
      format: json
      append: true

links:
  - from: code_input
    to: syntax_checker
    
  - from: syntax_checker
    to: security_reviewer
    params:
      pass_through:
        - syntax_result
    
  - from: security_reviewer
    to: performance_analyzer
    params:
      pass_through:
        - syntax_result
        - security_result
    
  - from: performance_analyzer
    to: result_merger
    
  - from: result_merger
    to: output
```

## How It Works

1. **Server Prefix Format**: When `model` contains a `/`, the prefix is treated as the server identifier
   - `localhost/llama3` → Uses llama3 from localhost:11434
   - `server1/devstral` → Uses devstral from the first saved Ollama server
   - `server2/deepseek-r1` → Uses deepseek from the second saved server

2. **Backward Compatibility**: You can still use the old format:
   ```yaml
   provider: local-ollama
   model: llama3:latest
   ```

3. **Server Resolution**: The server URLs are resolved from memory storage:
   - Searches for configs with type "arkavo_ollama_server_config"
   - Maps server1, server2, etc. to saved URLs in order

## Running the Pipeline

```bash
# Start the dataflow with the blueprint
arkavo dataflow start multi-server-blueprint.yaml

# Send code for review
curl -X POST http://localhost:8082/multi-review \
  -H "Content-Type: application/json" \
  -d '{
    "file": "example.py",
    "content": "def get_user(id): return db.query(f\"SELECT * FROM users WHERE id={id}\")"
  }'

# Results will show security issues from server1's model, 
# performance concerns from server2's model, etc.
```

## Benefits

- **Model Specialization**: Use the best model for each task
- **Load Distribution**: Spread work across multiple servers
- **Failover**: Can add logic to fallback if a server is unavailable
- **A/B Testing**: Compare outputs from different models/servers