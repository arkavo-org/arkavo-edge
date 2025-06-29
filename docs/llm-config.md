# LLM Configuration Guide

## HTTP Client Retry Configuration

The shared HTTP client infrastructure provides automatic retry logic with exponential backoff for transient failures.

### Default Retry Settings

| Parameter | Default Value | Description |
|-----------|--------------|-------------|
| Initial Delay | 1000ms | First retry delay |
| Max Retries | 3 | Maximum number of retry attempts |
| Backoff Factor | 2.0 | Exponential growth factor |
| Max Delay | 30s | Maximum delay between retries |
| Jitter Factor | 0.1 (10%) | Random jitter to prevent thundering herd |

### Retry Behavior

The retry delay calculation follows this pattern:
```
delay = min(initial_delay * (backoff_factor ^ attempt), max_delay)
jittered_delay = delay ± (delay * jitter_factor)
```

Example retry sequence:
- Attempt 1: ~1000ms (900-1100ms with jitter)
- Attempt 2: ~2000ms (1800-2200ms with jitter)  
- Attempt 3: ~4000ms (3600-4400ms with jitter)
- Attempt 4: ~8000ms (7200-8800ms with jitter)

### Provider-Specific Settings

Different providers may have different retry configurations:

#### OpenAI/Azure OpenAI
- Timeout: 60 seconds
- Max retries: 3
- Honors `Retry-After` headers for rate limits

#### Anthropic
- Timeout: 60 seconds
- Max retries: 3
- Handles Anthropic-specific rate limit errors

#### Ollama
- Timeout: 30 seconds
- Max retries: 3
- Optimized for local deployment

### Customizing Retry Behavior

Retry settings can be customized per provider in the Blueprint node configuration:

```json
{
  "id": "llm_node",
  "type": "llm",
  "params": {
    "provider": "openai",
    "model": "gpt-4",
    "timeout_secs": 120,
    "max_retries": 5,
    "initial_retry_delay_ms": 500,
    "backoff_factor": 1.5,
    "max_retry_delay_ms": 15000,
    "jitter_factor": 0.2
  }
}
```

### Error Classification

Errors are classified as retryable or non-retryable:

**Retryable Errors:**
- Rate limits (429)
- Server errors (500-599)
- Timeout errors
- Temporary network failures

**Non-Retryable Errors:**
- Authentication failures (401)
- Model not found (404)
- Invalid request (400)
- Insufficient quota

The system will only retry requests for retryable errors.