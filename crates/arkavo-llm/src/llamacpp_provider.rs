use crate::tool_parser::ToolParser;
use crate::{Error, Message, Provider, ProviderResponse, Result, Role, StreamResponse};
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llama_cpp::multimodal::MtmdContext;
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llama_cpp::{
    ChatInputs, ChatMessageMeta, LlamaModel, ModelFormat, apply_chat_template_with_format,
    detect_model_format, ffi, init_llama_logging, test_minimal_init,
};
use async_trait::async_trait;
use serde_json::Value;
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use std::ffi::CString;
use std::sync::Arc;
use tokio_stream::{Stream, wrappers::UnboundedReceiverStream};

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use crate::llamacpp_streaming::{StreamingConfig, generate_tokens};
use crate::mcp_converter::{LocalToolFormat, McpConverter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingMode {
    On,
    Off,
}

#[derive(Debug, Clone)]
pub struct SamplingConfig {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: i32,
    pub max_tokens: u32,
    pub seed: u32,
    pub debug: bool,
    /// Tool call format for local models (default: Fence for best small model reliability)
    pub tool_format: LocalToolFormat,
    /// Optional GBNF grammar for constrained tool call decoding
    pub grammar: Option<String>,
    /// Trigger patterns for lazy grammar activation (e.g., "```")
    pub grammar_triggers: Option<Vec<String>>,
    /// Explicit thinking mode override from autoresearch tuning
    pub thinking_mode: Option<ThinkingMode>,
    /// Tool definitions for native template rendering (passed to Jinja engine)
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub chat_tools: Vec<arkavo_llama_cpp::ChatTool>,
    /// Tool choice for native template rendering
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub chat_tool_choice: arkavo_llama_cpp::ToolChoice,
}

impl Default for SamplingConfig {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            top_p: 0.9,
            top_k: 40,
            max_tokens: 4096,
            seed: 42,
            debug: false,
            tool_format: LocalToolFormat::Fence,
            thinking_mode: None,
            grammar: None,
            grammar_triggers: None,
            #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
            chat_tools: Vec::new(),
            #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
            chat_tool_choice: arkavo_llama_cpp::ToolChoice::Auto,
        }
    }
}

use crate::ModelRegistry;

/// Type alias for conversation identifiers
type ConversationId = String;

/// Check if a model name indicates a sub-1B parameter model.
/// Sub-1B models lack capacity for useful chain-of-thought reasoning.
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
fn is_small_model(name: &str) -> bool {
    let lower = name.to_lowercase();
    // Match sub-1B size indicators: "0.6b", "0.8b", "270m", "500m", etc.
    lower.contains("0.6b")
        || lower.contains("0.8b")
        || lower.contains("270m")
        || lower.contains("500m")
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub struct LlamaCppProvider {
    /// Model reference - either owned directly or accessed via registry
    model: Option<Arc<LlamaModel>>,
    /// Model registry for multi-model mode
    registry: Option<Arc<ModelRegistry>>,
    /// Model name (key in registry for multi-model mode)
    name: String,
    config: SamplingConfig,
    mtmd_ctx: Option<Arc<MtmdContext>>,
    /// Optional conversation ID for context reuse
    conversation_id: Option<ConversationId>,
}

#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
pub struct LlamaCppProvider {
    name: String,
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
impl LlamaCppProvider {
    pub fn new(model_name: String, model_path: String) -> Result<Self> {
        Self::new_with_config(model_name, model_path, None, SamplingConfig::default())
    }

    pub fn new_with_mmproj(
        model_name: String,
        model_path: String,
        mmproj_path: String,
    ) -> Result<Self> {
        Self::new_with_config(
            model_name,
            model_path,
            Some(mmproj_path),
            SamplingConfig::default(),
        )
    }

    pub fn new_with_config(
        model_name: String,
        model_path: String,
        mmproj_path: Option<String>,
        config: SamplingConfig,
    ) -> Result<Self> {
        init_llama_logging();

        if config.debug {
            arkavo_llama_cpp::set_debug_logging(true);
            crate::llamacpp_streaming::set_debug(true);
        }

        test_minimal_init()
            .map_err(|e| Error::Config(format!("FFI initialization test failed: {e}")))?;

        let model = LlamaModel::from_file(&model_path)
            .map_err(|e| Error::Config(format!("Failed to load model: {e}")))?;

        let mtmd_ctx = if let Some(mmproj) = mmproj_path {
            let ctx = MtmdContext::from_file(&mmproj, &model)
                .map_err(|e| Error::Config(format!("Failed to load mmproj: {e}")))?;
            if !ctx.supports_vision() {
                return Err(Error::Config(
                    "mmproj model does not support vision".to_string(),
                ));
            }
            Some(Arc::new(ctx))
        } else {
            None
        };

        Ok(Self {
            model: Some(Arc::new(model)),
            registry: None,
            name: model_name,
            config,
            mtmd_ctx,
            conversation_id: None,
        })
    }

    /// Create a new provider that uses a ModelRegistry for multi-model support
    ///
    /// # Arguments
    /// * `registry` - The model registry containing loaded models
    /// * `model_name` - Name of the model to use from the registry
    /// * `config` - Sampling configuration
    pub fn new_with_registry(
        registry: Arc<ModelRegistry>,
        model_name: String,
        config: SamplingConfig,
    ) -> Result<Self> {
        // Verify the model exists in registry
        if !registry.is_loaded(&model_name) {
            return Err(Error::Config(format!(
                "Model '{model_name}' not found in registry"
            )));
        }

        Ok(Self {
            model: None,
            registry: Some(registry),
            name: model_name,
            config,
            mtmd_ctx: None,
            conversation_id: None,
        })
    }

    /// Create a provider with conversation context for multi-turn caching
    ///
    /// When a conversation_id is set, the provider will attempt to reuse
    /// the KV cache across turns for improved performance.
    pub fn new_with_conversation(
        registry: Arc<ModelRegistry>,
        model_name: String,
        conversation_id: ConversationId,
        config: SamplingConfig,
    ) -> Result<Self> {
        if !registry.is_loaded(&model_name) {
            return Err(Error::Config(format!(
                "Model '{model_name}' not found in registry"
            )));
        }

        Ok(Self {
            model: None,
            registry: Some(registry),
            name: model_name,
            config,
            mtmd_ctx: None,
            conversation_id: Some(conversation_id),
        })
    }

    /// Set the conversation ID for context reuse
    pub fn with_conversation(mut self, conversation_id: ConversationId) -> Self {
        self.conversation_id = Some(conversation_id);
        self
    }

    /// Get the current conversation ID
    pub fn conversation_id(&self) -> Option<&str> {
        self.conversation_id.as_deref()
    }

    /// Enable vision support by loading a multimodal projector file.
    ///
    /// Must be called after construction when the mmproj path is discovered
    /// separately (e.g., by the router's model discovery). Fails gracefully:
    /// logs a warning on load failure and returns Ok with text-only provider.
    pub fn enable_vision(mut self, mmproj_path: &str) -> Result<Self> {
        let model = self.get_model()?;
        match MtmdContext::from_file(mmproj_path, &model) {
            Ok(ctx) if ctx.supports_vision() => {
                tracing::info!("Vision support enabled via mmproj: {mmproj_path}");
                self.mtmd_ctx = Some(Arc::new(ctx));
            }
            Ok(_) => {
                tracing::warn!("mmproj loaded but does not support vision, continuing text-only");
            }
            Err(e) => {
                tracing::warn!("Failed to load mmproj ({mmproj_path}): {e}, continuing text-only");
            }
        }
        Ok(self)
    }

    /// Enable vision with a pre-loaded context (avoids reloading mmproj from disk).
    pub fn enable_vision_cached(mut self, ctx: Arc<MtmdContext>) -> Self {
        self.mtmd_ctx = Some(ctx);
        self
    }

    /// Get the vision context (for caching in the registry).
    pub fn vision_ctx(&self) -> Option<Arc<MtmdContext>> {
        self.mtmd_ctx.clone()
    }

    /// Get the model reference, either from owned or registry
    fn get_model(&self) -> Result<Arc<LlamaModel>> {
        if let Some(ref model) = self.model {
            Ok(model.clone())
        } else if let Some(ref registry) = self.registry {
            registry.get(&self.name).ok_or_else(|| {
                Error::Config(format!("Model '{}' not found in registry", self.name))
            })
        } else {
            Err(Error::Internal("Provider has no model source".to_string()))
        }
    }
}

#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
impl LlamaCppProvider {
    pub fn new(_model_name: String, _model_path: String) -> Result<Self> {
        Err(Error::Config(
            "llama-cpp feature not enabled - rebuild with --features llama-cpp".to_string(),
        ))
    }

    pub fn new_with_mmproj(
        _model_name: String,
        _model_path: String,
        _mmproj_path: String,
    ) -> Result<Self> {
        Err(Error::Config(
            "llama-cpp feature not enabled - rebuild with --features llama-cpp".to_string(),
        ))
    }

    pub fn new_with_config(
        _model_name: String,
        _model_path: String,
        _mmproj_path: Option<String>,
        _config: SamplingConfig,
    ) -> Result<Self> {
        Err(Error::Config(
            "llama-cpp feature not enabled - rebuild with --features llama-cpp".to_string(),
        ))
    }

    pub fn new_with_registry(
        _registry: Arc<ModelRegistry>,
        _model_name: String,
        _config: SamplingConfig,
    ) -> Result<Self> {
        Err(Error::Config(
            "llama-cpp feature not enabled - rebuild with --features llama-cpp".to_string(),
        ))
    }

    pub fn enable_vision(self, _mmproj_path: &str) -> Result<Self> {
        Ok(self)
    }
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
impl LlamaCppProvider {
    fn generate_streaming(
        &self,
        messages: Vec<Message>,
    ) -> Result<UnboundedReceiverStream<Result<StreamResponse>>> {
        let has_images = messages
            .iter()
            .any(|m| m.images.is_some() && !m.images.as_ref().unwrap().is_empty());

        if has_images && self.mtmd_ctx.is_some() {
            return self.generate_streaming_with_vision(messages);
        }

        let (llama_messages, _cstrings) = Self::messages_to_llama_chat_static(&messages)?;

        // Build per-message metadata for tool-role messages
        let meta: Vec<ChatMessageMeta> = messages
            .iter()
            .map(|m| ChatMessageMeta {
                tool_call_id: m.tool_call_id.clone(),
                tool_name: m.tool_name.clone(),
            })
            .collect();

        // Detect model format from model name
        let format = detect_model_format(&self.name);
        let model = self.get_model()?;

        // Disable thinking for sub-1B Qwen models (they lack capacity for CoT)
        let enable_thinking = match self.config.thinking_mode {
            Some(ThinkingMode::On) => true,
            Some(ThinkingMode::Off) => false,
            None => !(format == ModelFormat::Qwen3 && is_small_model(&self.name)),
        };

        // Try the Jinja template engine first (reads template from GGUF metadata),
        // fall back to legacy pattern-matched templates
        let (
            prompt_bytes,
            template_grammar,
            template_triggers,
            _thinking_forced_open,
            template_stops,
        ) = match model.chat_templates() {
            Ok(tmpls) => {
                let inputs = ChatInputs {
                    tools: self.config.chat_tools.clone(),
                    tool_choice: self.config.chat_tool_choice,
                    enable_thinking,
                    add_generation_prompt: true,
                };
                match tmpls.apply_with_meta(&llama_messages, &meta, &inputs) {
                    Ok(result) => {
                        if crate::llamacpp_streaming::is_debug() {
                            if let Ok(s) = std::str::from_utf8(&result.prompt) {
                                eprintln!("Chat template output (Jinja):\n{s}");
                            }
                            eprintln!(
                                "✓ Template from GGUF metadata (enable_thinking={enable_thinking})"
                            );
                            if result.grammar.is_some() {
                                eprintln!(
                                    "  grammar: {} bytes, lazy={}",
                                    result.grammar.as_ref().map_or(0, |g| g.len()),
                                    result.grammar_lazy
                                );
                            }
                            if result.thinking_forced_open {
                                eprintln!("  thinking_forced_open=true");
                            }
                        }
                        // Template grammar uses character-level GBNF rules (e.g., "<tool_call>")
                        // but models tokenize these as single special tokens. This mismatch
                        // causes GGML_ASSERT failures in the grammar sampler. The template
                        // grammar is designed for llama-server's integrated grammar handler
                        // which resolves special tokens — our standalone sampler cannot use it.
                        // We rely on the template's prompt formatting + enable_thinking=false
                        // to guide generation, and use our own fence grammar if configured.
                        (
                            result.prompt,
                            None,
                            None,
                            result.thinking_forced_open,
                            result.additional_stops,
                        )
                    }
                    Err(e) => {
                        tracing::warn!("Jinja template apply failed: {e}, falling back to legacy");
                        let bytes = apply_chat_template_with_format(&llama_messages, true, format)
                            .map_err(|e| {
                                Error::Config(format!("Failed to apply chat template: {e}"))
                            })?;
                        (bytes, None, None, false, Vec::new())
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Chat templates init failed: {e}, falling back to legacy");
                let bytes = apply_chat_template_with_format(&llama_messages, true, format)
                    .map_err(|e| Error::Config(format!("Failed to apply chat template: {e}")))?;
                (bytes, None, None, false, Vec::new())
            }
        };

        if crate::llamacpp_streaming::is_debug()
            && let Ok(prompt_str) = std::str::from_utf8(&prompt_bytes)
        {
            // Only show legacy format checks when using legacy path
            if template_grammar.is_none() {
                eprintln!("Chat template output:\n{prompt_str}");
            }
        }

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        // Enable dry sampling for repetition prevention (all models can loop)
        let use_dry_sampling = true;

        // Merge template grammar with config grammar (template takes precedence)
        let grammar = template_grammar.or_else(|| self.config.grammar.clone());
        let grammar_triggers_merged =
            template_triggers.or_else(|| self.config.grammar_triggers.clone());

        let additional_stops = template_stops;

        let streaming_config = StreamingConfig {
            temperature: self.config.temperature,
            top_p: self.config.top_p,
            top_k: self.config.top_k,
            max_tokens: self.config.max_tokens,
            seed: self.config.seed,
            use_dry_sampling,
            model_format: format,
            grammar,
            grammar_triggers: grammar_triggers_merged,
            additional_stops,
        };

        tokio::spawn(async move {
            generate_tokens(model, prompt_bytes, streaming_config, tx).await;
        });

        Ok(UnboundedReceiverStream::new(rx))
    }

    fn generate_streaming_with_vision(
        &self,
        messages: Vec<Message>,
    ) -> Result<UnboundedReceiverStream<Result<StreamResponse>>> {
        use crate::llamacpp_streaming::generate_tokens_with_vision;

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let model = self.get_model()?;
        let mtmd_ctx = self
            .mtmd_ctx
            .clone()
            .ok_or_else(|| Error::Config("Vision context not initialized".to_string()))?;

        // Detect if GLM for dry sampling
        let format = detect_model_format(&self.name);
        let use_dry_sampling = matches!(format, ModelFormat::GLM4);

        let streaming_config = StreamingConfig {
            temperature: self.config.temperature,
            top_p: self.config.top_p,
            top_k: self.config.top_k,
            max_tokens: self.config.max_tokens,
            seed: self.config.seed,
            use_dry_sampling,
            model_format: format,
            grammar: None,
            grammar_triggers: None,
            additional_stops: Vec::new(),
        };

        tokio::spawn(async move {
            generate_tokens_with_vision(model, mtmd_ctx, messages, streaming_config, tx).await;
        });

        Ok(UnboundedReceiverStream::new(rx))
    }

    async fn complete_with_timing(
        &self,
        messages: Vec<Message>,
        max_tokens: Option<usize>,
    ) -> Result<(String, Option<crate::provider::InferenceTiming>)> {
        let custom_provider;
        let provider = if let Some(max) = max_tokens {
            let mut config = self.config.clone();
            config.max_tokens = max as u32;
            custom_provider = Self {
                model: self.model.clone(),
                registry: self.registry.clone(),
                name: self.name.clone(),
                config,
                mtmd_ctx: self.mtmd_ctx.clone(),
                conversation_id: self.conversation_id.clone(),
            };
            &custom_provider
        } else {
            self
        };

        let mut stream = provider.generate_streaming(messages)?;
        let mut full_response = String::new();
        let mut timing = None;

        while let Some(chunk) = tokio_stream::StreamExt::next(&mut stream).await {
            match chunk {
                Ok(response) => {
                    full_response.push_str(&response.content);
                    if response.done {
                        timing = response.inference_timing;
                        break;
                    }
                }
                Err(e) => return Err(e),
            }
        }

        Ok((full_response, timing))
    }

    fn messages_to_llama_chat_static(
        messages: &[Message],
    ) -> Result<(Vec<ffi::llama_chat_message>, Vec<CString>)> {
        let mut llama_messages = Vec::new();
        let mut role_strings = Vec::new();
        let mut content_strings = Vec::new();

        for msg in messages {
            let role_str = match msg.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            };

            let role_cstring = CString::new(role_str)
                .map_err(|e| Error::Config(format!("Invalid role string: {e}")))?;
            let content_cstring = CString::new(msg.content.clone())
                .map_err(|e| Error::Config(format!("Invalid content string: {e}")))?;

            llama_messages.push(ffi::llama_chat_message {
                role: role_cstring.as_ptr(),
                content: content_cstring.as_ptr(),
            });

            role_strings.push(role_cstring);
            content_strings.push(content_cstring);
        }

        let mut all_cstrings = role_strings;
        all_cstrings.extend(content_strings);

        Ok((llama_messages, all_cstrings))
    }
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[async_trait]
impl Provider for LlamaCppProvider {
    async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        max_tokens: Option<usize>,
    ) -> Result<String> {
        let custom_provider;
        let provider = if let Some(max) = max_tokens {
            let mut config = self.config.clone();
            config.max_tokens = max as u32;
            custom_provider = Self {
                model: self.model.clone(),
                registry: self.registry.clone(),
                name: self.name.clone(),
                config,
                mtmd_ctx: self.mtmd_ctx.clone(),
                conversation_id: self.conversation_id.clone(),
            };
            &custom_provider
        } else {
            self
        };

        let mut stream = provider.generate_streaming(messages)?;
        let mut full_response = String::new();

        while let Some(chunk) = tokio_stream::StreamExt::next(&mut stream).await {
            match chunk {
                Ok(response) => {
                    full_response.push_str(&response.content);
                    if response.done {
                        break;
                    }
                }
                Err(e) => return Err(e),
            }
        }

        Ok(full_response)
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        let stream = self.generate_streaming(messages)?;
        Ok(Box::new(stream))
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn supports_tools(&self) -> bool {
        true
    }

    async fn complete_with_tools(
        &self,
        messages: Vec<Message>,
        tools: Option<Value>,
        max_tokens: Option<usize>,
    ) -> Result<ProviderResponse> {
        // Detect GLM model for special handling
        let format = detect_model_format(&self.name);
        let is_glm = matches!(format, ModelFormat::GLM4);

        let mut tool_grammar: Option<(String, Vec<String>)> = None;

        let system_prompt = if let Some(tools_value) = tools.as_ref() {
            let tools_array = tools_value
                .as_array()
                .ok_or_else(|| Error::Provider("Tools must be an array".into()))?;

            let tool_infos: Vec<arkavo_mcp_tools::registry::ToolInfo> = tools_array
                .iter()
                .filter_map(|t| {
                    Some(arkavo_mcp_tools::registry::ToolInfo {
                        name: t.get("name")?.as_str()?.to_string(),
                        description: t.get("description")?.as_str()?.to_string(),
                        schema: t.get("input_schema")?.clone(),
                        category: "general".to_string(),
                    })
                })
                .collect();

            // Generate GBNF grammar for fence-format tool calls when explicitly enabled.
            // Grammar enforcement is opt-in via SamplingConfig because lazy grammar
            // triggers can cause crashes on some model/quant combinations.
            if self.config.grammar.is_some()
                && matches!(self.config.tool_format, LocalToolFormat::Fence)
                && !tool_infos.is_empty()
            {
                let (grammar, _root) =
                    crate::tool_grammar::fence_grammar_after_trigger(&tool_infos);
                let triggers: Vec<String> = crate::tool_grammar::fence_trigger_patterns()
                    .into_iter()
                    .map(String::from)
                    .collect();
                tool_grammar = Some((grammar, triggers));
            }

            // Use GLM-specific prompt that emphasizes tools are optional
            if is_glm {
                McpConverter::to_glm_prompt(&tool_infos)
            } else {
                McpConverter::to_local_prompt(&tool_infos, self.config.tool_format)
            }
        } else {
            String::new()
        };

        let mut modified_messages = messages.clone();
        if !system_prompt.is_empty() {
            if let Some(first) = modified_messages.first_mut() {
                if first.role == Role::System {
                    first.content = format!("{}\n\n{}", system_prompt, first.content);
                } else {
                    modified_messages.insert(0, Message::system(system_prompt));
                }
            } else {
                modified_messages.push(Message::system(system_prompt));
            }
        }

        // Model-specific temperature tuning for tool calling reliability
        let tool_temperature = if tools.is_some() {
            let name_lower = self.name.to_lowercase();
            if is_glm {
                Some(0.15)
            } else if name_lower.contains("0.6b")
                || name_lower.contains("0.8b")
                || name_lower.contains("270m")
            {
                Some(0.1) // Near-greedy for tiny models
            } else if name_lower.contains("3b") || name_lower.contains("4b") {
                Some(0.2)
            } else {
                None // Keep default for 8B+
            }
        } else {
            None
        };

        // Convert tool definitions to ChatTool format for native template rendering
        let chat_tools: Vec<arkavo_llama_cpp::ChatTool> = if let Some(ref tools_value) = tools {
            tools_value
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|t| {
                            Some(arkavo_llama_cpp::ChatTool {
                                name: t.get("name")?.as_str()?.to_string(),
                                description: t
                                    .get("description")
                                    .and_then(|d| d.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                parameters_json: t
                                    .get("input_schema")
                                    .map(|s| s.to_string())
                                    .unwrap_or_default(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let (raw_content, inference_timing) = {
            let mut config = self.config.clone();
            if let Some(temp) = tool_temperature {
                config.temperature = temp;
            }
            if let Some((grammar, triggers)) = tool_grammar {
                config.grammar = Some(grammar);
                config.grammar_triggers = Some(triggers);
            }
            // Pass tools to the Jinja template engine for native grammar generation
            if !chat_tools.is_empty() {
                config.chat_tools = chat_tools;
                config.chat_tool_choice = arkavo_llama_cpp::ToolChoice::Required;
            }
            let custom_provider = Self {
                model: self.model.clone(),
                registry: self.registry.clone(),
                name: self.name.clone(),
                config,
                mtmd_ctx: self.mtmd_ctx.clone(),
                conversation_id: self.conversation_id.clone(),
            };
            custom_provider
                .complete_with_timing(modified_messages, max_tokens)
                .await?
        };

        // Extract thinking blocks for GLM models; also strip for sub-1B Qwen defensively
        let (content, reasoning_content) = if is_glm {
            let extraction = ToolParser::extract_thinking_blocks(&raw_content);
            let reasoning = if extraction.thinking.is_empty() {
                None
            } else {
                Some(extraction.thinking)
            };
            (extraction.content, reasoning)
        } else if format == ModelFormat::Qwen3 && is_small_model(&self.name) {
            let extraction = ToolParser::extract_thinking_blocks(&raw_content);
            (extraction.content, None)
        } else {
            (raw_content, None)
        };

        let tool_calls = if let Some(ref tools_value) = tools {
            // Collect registered tool names to filter false positives
            // (e.g. ```python``` code fences matching as tool calls)
            let registered_names: std::collections::HashSet<&str> = tools_value
                .as_array()
                .map(|arr| arr.iter().filter_map(|t| t.get("name")?.as_str()).collect())
                .unwrap_or_default();

            // Try configured format first, then fallback chain
            let parsed = match self.config.tool_format {
                LocalToolFormat::Fence => ToolParser::parse_fence(&content)
                    .or_else(|_| ToolParser::parse_xml(&content))
                    .unwrap_or_default(),
                LocalToolFormat::Xml => ToolParser::parse_xml(&content)
                    .or_else(|_| ToolParser::parse_fence(&content))
                    .unwrap_or_default(),
                LocalToolFormat::Json => ToolParser::parse_json(&content)
                    .or_else(|_| ToolParser::parse_fence(&content))
                    .unwrap_or_default(),
            };

            // Only keep calls that match actual registered tools
            parsed
                .into_iter()
                .filter(|c| registered_names.contains(c.tool_name.as_str()))
                .collect()
        } else {
            Vec::new()
        };

        // Strip tool call syntax from displayed content when tools were parsed
        let display_content = if !tool_calls.is_empty() {
            ToolParser::strip_fence_blocks(&content)
        } else {
            content
        };

        Ok(ProviderResponse {
            content: display_content,
            reasoning_content,
            tool_calls,
            finish_reason: None,
            inference_timing,
        })
    }
}

/// Check if GPU acceleration is available for local inference
///
/// Returns `true` if GPU is available or status is unknown (not yet tested).
/// Returns `false` only if GPU has been tested and failed.
///
/// This is used by the router to make hardware-aware model selection decisions.
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub fn is_gpu_accelerated() -> bool {
    use arkavo_llama_cpp::{GpuStatus, gpu_status};
    matches!(gpu_status(), GpuStatus::Available | GpuStatus::Unknown)
}

/// Stub for when llama-cpp is disabled - always returns false
#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
pub fn is_gpu_accelerated() -> bool {
    false
}

#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
#[async_trait]
impl Provider for LlamaCppProvider {
    async fn complete_with_options(
        &self,
        _messages: Vec<Message>,
        _max_tokens: Option<usize>,
    ) -> Result<String> {
        Err(Error::Config(
            "llama-cpp feature not enabled - rebuild with --features llama-cpp".to_string(),
        ))
    }

    async fn stream(
        &self,
        _messages: Vec<Message>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        Err(Error::Config(
            "llama-cpp feature not enabled - rebuild with --features llama-cpp".to_string(),
        ))
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn supports_tools(&self) -> bool {
        false
    }

    async fn complete_with_tools(
        &self,
        _messages: Vec<Message>,
        _tools: Option<Value>,
        _max_tokens: Option<usize>,
    ) -> Result<ProviderResponse> {
        Err(Error::Config(
            "llama-cpp feature not enabled - rebuild with --features llama-cpp".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    use super::is_small_model;

    #[test]
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    fn test_is_small_model() {
        assert!(is_small_model("qwen3.5-0.8b"));
        assert!(is_small_model("Qwen3-0.6B"));
        assert!(is_small_model("gemma-3-270m-it"));
        assert!(is_small_model("custom-500m-model"));

        assert!(!is_small_model("qwen3.5-27b"));
        assert!(!is_small_model("ministral-3b"));
        assert!(!is_small_model("ministral-8b"));
        assert!(!is_small_model("glm-4.7-flash"));
    }
}
