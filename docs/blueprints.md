# Consolidated Arkavo Dataflow Blueprints

This document consolidates all blueprint examples from the `/examples/blueprints` directory.

## Overview

This directory contains example blueprints for reference. 

**For AI Agents**: See `/docs/LLM_DATAFLOW_GUIDE.md` for comprehensive documentation on discovering, configuring, and using LLM capabilities through MCP tools.

**For Humans**: These examples are automatically discoverable by the AI agent. You don't need to manually configure anything - the agent will handle LLM provider discovery and blueprint generation based on your requirements.

## Blueprint Examples

### LLM Summarizer (llm-summarizer.json)

Basic text summarization pipeline that receives data via webhook and summarizes it using a local Ollama instance.

```json
{
  "version": "1.0.0",
  "name": "llm-summarizer",
  "description": "Pipeline that summarizes incoming JSON data using LLM",
  "nodes": [
    {
      "id": "webhook_input",
      "kind": "source",
      "params": {
        "type": "webhook_source",
        "port": 8080,
        "path": "/data"
      }
    },
    {
      "id": "summarizer",
      "kind": "transform",
      "params": {
        "type": "llm_transform",
        "provider": "local-ollama",
        "model": "llama3.2:latest",
        "prompt": "Summarize the following data in 2-3 sentences:\n\n{{input}}",
        "temperature": 0.5,
        "max_tokens": 150
      }
    },
    {
      "id": "output",
      "kind": "sink",
      "params": {
        "type": "console_sink",
        "format": "json"
      }
    }
  ],
  "links": [
    {
      "from": "webhook_input",
      "to": "summarizer"
    },
    {
      "from": "summarizer",
      "to": "output"
    }
  ]
}
```

### LLM Code Reviewer (llm-code-reviewer.yaml)

Pipeline that reviews code changes using a code-focused LLM model. This example is in YAML format to show format flexibility.

```yaml
version: 1.0.0
name: llm-code-reviewer
description: Pipeline that reviews code changes using a code-focused LLM model

nodes:
  - id: code_input
    kind: source
    params:
      type: webhook_source
      port: 8081
      path: /review
      
  - id: code_formatter
    kind: transform
    params:
      type: json_transform
      spec:
        filename: "file.name"
        language: "file.language"
        content: "file.content"
        
  - id: code_reviewer
    kind: transform
    params:
      type: llm_transform
      provider: local-ollama
      model: devstral:latest
      prompt: |
        Review the following code and provide feedback on:
        1. Code quality and best practices
        2. Potential bugs or issues
        3. Performance considerations
        4. Security concerns
        
        Code:
        {{input}}
      temperature: 0.2
      max_tokens: 500
      
  - id: review_enricher
    kind: transform
    params:
      type: enrich_transform
      fields:
        review_timestamp: "${CURRENT_TIME}"
        reviewer: "arkavo-llm"
      add_metadata: true
      
  - id: review_output
    kind: sink
    params:
      type: file_sink
      path: code_reviews.json
      format: json
      append: true

links:
  - from: code_input
    to: code_formatter
    
  - from: code_formatter
    to: code_reviewer
    
  - from: code_reviewer
    to: review_enricher
    
  - from: review_enricher
    to: review_output
```

### LLM Router Pattern (llm-router-pattern.json)

Pipeline demonstrating LLM-based routing based on content classification. Uses a lightweight model to classify content, then routes to appropriate specialized processors.

```json
{
  "version": "1.0.0",
  "name": "llm-router-pattern",
  "description": "Pipeline demonstrating LLM-based routing based on content classification",
  "nodes": [
    {
      "id": "input_stream",
      "kind": "source",
      "params": {
        "type": "webhook_source",
        "port": 8082,
        "path": "/classify"
      }
    },
    {
      "id": "classifier",
      "kind": "transform",
      "params": {
        "type": "llm_transform",
        "provider": "local-ollama",
        "model": "qwen3:0.6b",
        "prompt": "Classify the following text into one category: technical, business, or general. Respond with only the category name.\n\nText: {{input}}",
        "temperature": 0.1,
        "max_tokens": 10
      }
    },
    {
      "id": "router",
      "kind": "router",
      "params": {
        "type": "conditional_router",
        "field": "llm_response"
      }
    },
    {
      "id": "technical_processor",
      "kind": "transform",
      "params": {
        "type": "llm_transform",
        "provider": "local-ollama",
        "model": "devstral:latest",
        "prompt": "Provide a technical analysis of: {{input}}",
        "temperature": 0.3
      }
    },
    {
      "id": "business_processor",
      "kind": "transform",
      "params": {
        "type": "llm_transform",
        "provider": "local-ollama",
        "model": "llama3.2:latest",
        "prompt": "Provide a business impact analysis of: {{input}}",
        "temperature": 0.5
      }
    },
    {
      "id": "general_processor",
      "kind": "transform",
      "params": {
        "type": "llm_transform",
        "provider": "local-ollama",
        "model": "qwen3:latest",
        "prompt": "Summarize the following text: {{input}}",
        "temperature": 0.7
      }
    },
    {
      "id": "output_sink",
      "kind": "sink",
      "params": {
        "type": "console_sink",
        "format": "pretty"
      }
    }
  ],
  "links": [
    {
      "from": "input_stream",
      "to": "classifier"
    },
    {
      "from": "classifier",
      "to": "router"
    },
    {
      "from": "router",
      "to": "technical_processor",
      "rule": {
        "type": "filter",
        "conditions": [
          {
            "field": "llm_response",
            "operator": "contains",
            "value": "technical"
          }
        ]
      }
    },
    {
      "from": "router",
      "to": "business_processor",
      "rule": {
        "type": "filter",
        "conditions": [
          {
            "field": "llm_response",
            "operator": "contains",
            "value": "business"
          }
        ]
      }
    },
    {
      "from": "router",
      "to": "general_processor",
      "rule": {
        "type": "filter",
        "conditions": [
          {
            "field": "llm_response",
            "operator": "contains",
            "value": "general"
          }
        ]
      }
    },
    {
      "from": "technical_processor",
      "to": "output_sink"
    },
    {
      "from": "business_processor",
      "to": "output_sink"
    },
    {
      "from": "general_processor",
      "to": "output_sink"
    }
  ]
}
```

### LLM Multi-Provider (llm-multi-provider.json)

Pipeline demonstrating multiple Ollama instances for different tasks. Shows how to use both local and remote Ollama providers in parallel.

```json
{
  "version": "1.0.0",
  "name": "llm-multi-provider",
  "description": "Pipeline demonstrating multiple Ollama instances for different tasks",
  "nodes": [
    {
      "id": "timer_input",
      "kind": "source",
      "params": {
        "type": "timer_source",
        "interval_ms": 5000,
        "message": {
          "task": "analyze",
          "content": "The quick brown fox jumps over the lazy dog. This is a test sentence for analysis."
        }
      }
    },
    {
      "id": "local_analysis",
      "kind": "transform",
      "params": {
        "type": "llm_transform",
        "provider": "local-ollama",
        "model": "llama3:latest",
        "task_type": "sentiment_analysis",
        "prompt": "Analyze the sentiment and key themes in this text: {{input}}",
        "temperature": 0.7
      }
    },
    {
      "id": "remote_translation",
      "kind": "transform",
      "params": {
        "type": "llm_transform",
        "provider": "remote-ollama",
        "model": "devstral:latest",
        "task_type": "translation",
        "prompt": "Translate the following text to Spanish: {{input}}",
        "temperature": 0.3,
        "auth_ref": "${REMOTE_OLLAMA_API_KEY}"
      }
    },
    {
      "id": "merger",
      "kind": "transform",
      "params": {
        "type": "json_transform",
        "spec": {
          "original": "original",
          "local_result": "llm_response",
          "remote_result": "llm_response"
        }
      }
    },
    {
      "id": "file_output",
      "kind": "sink",
      "params": {
        "type": "file_sink",
        "path": "multi_llm_results.jsonl",
        "format": "jsonl"
      }
    }
  ],
  "links": [
    {
      "from": "timer_input",
      "to": "local_analysis"
    },
    {
      "from": "timer_input",
      "to": "remote_translation"
    },
    {
      "from": "local_analysis",
      "to": "merger"
    },
    {
      "from": "remote_translation",
      "to": "merger"
    },
    {
      "from": "merger",
      "to": "file_output"
    }
  ],
  "metadata": {
    "remote_ollama_url": "http://10.0.0.101:11434",
    "description": "This pipeline demonstrates using two different Ollama instances - one local for analysis and one remote for translation"
  }
}
```

## Key Concepts

### Node Types

1. **Source Nodes** - Data ingestion points (webhook, timer, file watcher, etc.)
2. **Transform Nodes** - Data processing including LLM transforms
3. **Router Nodes** - Conditional routing based on data content
4. **Sink Nodes** - Data output destinations (console, file, database, etc.)

### LLM Transform Parameters

- **provider**: The LLM provider to use (e.g., "local-ollama", "remote-ollama")
- **model**: The specific model to use (e.g., "llama3.2:latest", "devstral:latest")
- **prompt**: The prompt template with {{input}} placeholder
- **temperature**: Controls randomness (0.0 = deterministic, 1.0 = creative)
- **max_tokens**: Maximum response length
- **task_type**: Optional categorization for the task

### Common Patterns

1. **Simple Processing**: Source → LLM Transform → Sink
2. **Multi-Stage**: Source → Transform → LLM → Enrich → Sink
3. **Routing**: Source → Classifier LLM → Router → Specialized LLMs → Sink
4. **Parallel Processing**: Source → Multiple LLMs in parallel → Merger → Sink

## Usage

These blueprints are automatically discoverable by the AI agent. When you need LLM capabilities:

1. The agent will discover available providers using MCP tools
2. Generate appropriate blueprints based on your requirements
3. Deploy and manage the dataflow pipelines

No manual configuration needed - just describe what you want to accomplish!