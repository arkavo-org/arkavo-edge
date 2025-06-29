# Implement Multi-Model and Multi-Provider Support for Arkavo LLM Architecture

## Problem Statement

The current Arkavo LLM architecture has significant limitations that prevent effective multi-model and multi-provider support:

### Current Limitations

1. **Single Provider Architecture**: The system can only use one LLM provider at a time, determined at initialization via environment variables (`LLM_PROVIDER`).

2. **No Dynamic Model Selection**: Model selection is hardcoded within the Ollama client based on simple heuristics (vision, coding, general tasks) rather than allowing explicit model specification per request.

3. **Limited Provider Support**: Only Ollama is currently implemented, with no support for frontier model providers (OpenAI, Anthropic, Google, etc.).

4. **Inflexible Request Routing**: The `LlmClient` uses a boxed trait object that makes it impossible to route requests to different providers dynamically.

5. **Poor Model Discovery**: No mechanism exists to discover available models across multiple providers or understand their capabilities.

6. **Configuration Management**: API keys and endpoints are managed through environment variables with no support for multiple concurrent configurations.

7. **UI-LLM Communication Protocol**: The terminal UI uses a string-based protocol that doesn't support model specification in requests.

## Requirements

### Functional Requirements

1. **Multi-Provider Support**
   - Support multiple LLM providers simultaneously (Ollama, OpenAI, Anthropic, Google, etc.)
   - Allow adding new providers without modifying core architecture
   - Enable provider-specific features and configurations

2. **Dynamic Model Selection**
   - Allow specifying the model for each request
   - Support model aliases for convenience (e.g., "fast", "smart", "vision")
   - Enable fallback models when primary choice is unavailable

3. **Model Discovery and Capabilities**
   - Discover available models from each provider
   - Track model capabilities (context length, vision support, function calling, etc.)
   - Monitor model availability and health

4. **Request Routing**
   - Route requests to appropriate provider/model based on requirements
   - Support load balancing across multiple instances
   - Handle provider failures gracefully with fallbacks

5. **Configuration Management**
   - Support multiple API keys and endpoints
   - Allow runtime configuration updates
   - Secure storage of sensitive credentials

### Non-Functional Requirements

1. **Performance**: Minimal overhead for request routing (< 1ms)
2. **Reliability**: Automatic fallback when providers are unavailable
3. **Extensibility**: Easy to add new providers
4. **Backward Compatibility**: Existing code should continue to work
5. **Type Safety**: Leverage Rust's type system for compile-time guarantees

## Proposed Architecture

### 1. Core Components

```rust
// Model identifier that includes provider and model name
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelId {
    pub provider: String,
    pub model: String,
}

// Model capabilities
#[derive(Debug, Clone)]
pub struct ModelCapabilities {
    pub context_length: usize,
    pub supports_vision: bool,
    pub supports_function_calling: bool,
    pub supports_streaming: bool,
    pub cost_per_token: Option<f64>,
}

// Model information
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub id: ModelId,
    pub display_name: String,
    pub capabilities: ModelCapabilities,
    pub available: bool,
}

// Provider trait with async model discovery
#[async_trait]
pub trait Provider: Send + Sync {
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;
    async fn complete(&self, model: &str, messages: Vec<Message>) -> Result<String>;
    async fn stream(&self, model: &str, messages: Vec<Message>) 
        -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>>;
    fn name(&self) -> &str;
    fn validate_config(&self) -> Result<()>;
}
```

### 2. Router Architecture

```rust
pub struct LlmRouter {
    providers: HashMap<String, Box<dyn Provider>>,
    model_registry: Arc<RwLock<ModelRegistry>>,
    default_model: Option<ModelId>,
    fallback_chain: Vec<ModelId>,
}

impl LlmRouter {
    pub async fn complete(&self, request: CompletionRequest) -> Result<String> {
        let model_id = request.model.unwrap_or_else(|| self.select_model(&request));
        let provider = self.get_provider(&model_id)?;
        
        match provider.complete(&model_id.model, request.messages).await {
            Ok(response) => Ok(response),
            Err(e) => self.try_fallback(request, e).await,
        }
    }
    
    fn select_model(&self, request: &CompletionRequest) -> ModelId {
        // Intelligent model selection based on request characteristics
        // Consider: message content, image presence, context length, etc.
    }
}
```

### 3. Configuration System

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    pub providers: HashMap<String, ProviderConfig>,
    pub routing: RoutingConfig,
    pub models: HashMap<String, ModelAlias>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum ProviderConfig {
    Ollama {
        base_url: String,
        models: Vec<String>,
    },
    OpenAI {
        api_key: String,
        organization_id: Option<String>,
        base_url: Option<String>,
    },
    Anthropic {
        api_key: String,
        base_url: Option<String>,
    },
    // ... other providers
}
```

### 4. Enhanced Terminal UI Protocol

```rust
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub task_id: Uuid,
    pub model_name: String,  // Can be ModelId or alias
    pub prompt: String,
    pub options: RequestOptions,
}

#[derive(Debug, Clone)]
pub struct RequestOptions {
    pub temperature: Option<f32>,
    pub max_tokens: Option<usize>,
    pub require_vision: bool,
    pub require_function_calling: bool,
}
```

## Implementation Plan

### Phase 1: Core Architecture Refactoring (Week 1-2)

1. **Create new crate `arkavo-llm-router`**
   - Define core traits and types
   - Implement model registry
   - Create router implementation

2. **Refactor existing provider trait**
   - Add model parameter to methods
   - Add capability discovery
   - Implement provider validation

3. **Update Ollama provider**
   - Support explicit model selection
   - Implement proper model discovery
   - Add capability detection

### Phase 2: Configuration and Management (Week 3)

4. **Implement configuration system**
   - Create configuration types
   - Add configuration loader
   - Implement secure credential storage

5. **Add model registry**
   - Track available models
   - Monitor model health
   - Cache model capabilities

6. **Create model selection logic**
   - Implement intelligent routing
   - Add fallback mechanisms
   - Support model aliases

### Phase 3: Provider Implementation (Week 4-5)

7. **Implement OpenAI provider**
   - API client implementation
   - Model discovery
   - Streaming support

8. **Implement Anthropic provider**
   - API client implementation
   - Model discovery
   - Streaming support

9. **Add other providers**
   - Google (Gemini)
   - Mistral
   - Local providers (llama.cpp, etc.)

### Phase 4: UI and Integration (Week 6)

10. **Update Terminal UI**
    - Enhance protocol for model selection
    - Add model picker UI
    - Display model information

11. **Update CLI commands**
    - Add model selection flags
    - Support provider configuration
    - Add model listing command

12. **Integration testing**
    - Test multi-provider scenarios
    - Verify fallback mechanisms
    - Performance benchmarking

## Migration Strategy

### 1. Backward Compatibility

- Keep existing `LlmClient` as a facade over the new router
- Support existing environment variables
- Provide migration warnings for deprecated usage

### 2. Gradual Migration Path

```rust
// Phase 1: Add router alongside existing client
pub struct LlmClient {
    provider: Box<dyn Provider>, // Keep for compatibility
    router: Option<LlmRouter>,   // New router (optional initially)
}

// Phase 2: Route through router when available
impl LlmClient {
    pub async fn complete(&self, messages: Vec<Message>) -> Result<String> {
        if let Some(router) = &self.router {
            router.complete(CompletionRequest {
                model: None, // Use default
                messages,
                options: Default::default(),
            }).await
        } else {
            self.provider.complete(messages).await // Legacy path
        }
    }
}

// Phase 3: Deprecate direct provider usage
```

### 3. Configuration Migration

1. Start with environment variables (current state)
2. Add configuration file support
3. Provide migration tool for existing setups
4. Eventually deprecate environment-only configuration

## Testing Strategy

### Unit Tests
- Provider implementations
- Router logic
- Model selection algorithms
- Configuration parsing

### Integration Tests
- Multi-provider scenarios
- Fallback mechanisms
- Streaming across providers
- Error handling

### End-to-End Tests
- Terminal UI with multiple models
- CLI commands with model selection
- Configuration updates
- Provider failures

## Success Criteria

1. **Multiple Providers**: Successfully use 3+ providers simultaneously
2. **Dynamic Selection**: Route requests to different models based on content
3. **Fallback Working**: Automatic fallback when primary model unavailable
4. **Performance**: < 1ms routing overhead
5. **User Experience**: Seamless model selection in Terminal UI
6. **Extensibility**: Add new provider in < 100 lines of code

## Future Enhancements

1. **Cost Optimization**: Route based on cost/performance trade-offs
2. **A/B Testing**: Compare model outputs for same prompts
3. **Caching**: Cache responses for identical requests
4. **Monitoring**: Track model performance and usage
5. **Rate Limiting**: Respect provider rate limits
6. **Custom Models**: Support for fine-tuned and custom models

## Technical Debt to Address

1. Remove string-based protocol between UI and LLM
2. Eliminate environment variable dependencies
3. Replace boxed trait objects with enum dispatch where possible
4. Add proper error types for each provider
5. Implement connection pooling for HTTP clients

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Breaking existing code | High | Maintain backward compatibility layer |
| Provider API changes | Medium | Abstract provider-specific details |
| Performance regression | Medium | Benchmark critical paths |
| Configuration complexity | Medium | Provide sensible defaults |
| Security of API keys | High | Use OS keychain integration |

## References

- [OpenAI API](https://platform.openai.com/docs/api-reference)
- [Anthropic API](https://docs.anthropic.com/claude/reference/getting-started-with-the-api)
- [Google Gemini API](https://ai.google.dev/docs)
- [Ollama API](https://github.com/ollama/ollama/blob/main/docs/api.md)