use crate::tool_parser::ToolParser;
use crate::{Error, Message, Provider, ProviderResponse, Result, Role, StreamResponse};
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llama_cpp::multimodal::MtmdContext;
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llama_cpp::{
    LlamaModel, ModelFormat, apply_chat_template_with_format, detect_model_format, ffi,
    init_llama_logging, test_minimal_init,
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
        }
    }
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub struct LlamaCppProvider {
    model: Arc<LlamaModel>,
    name: String,
    config: SamplingConfig,
    mtmd_ctx: Option<Arc<MtmdContext>>,
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
            model: Arc::new(model),
            name: model_name,
            config,
            mtmd_ctx,
        })
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

        // Detect model format from model name
        let format = detect_model_format(&self.name);

        let prompt_bytes = apply_chat_template_with_format(&llama_messages, true, format)
            .map_err(|e| Error::Config(format!("Failed to apply chat template: {e}")))?;

        if crate::llamacpp_streaming::is_debug()
            && let Ok(prompt_str) = std::str::from_utf8(&prompt_bytes)
        {
            eprintln!("Chat template output:\n{prompt_str}");
            match format {
                ModelFormat::Gemma3 => {
                    if prompt_str.contains("<start_of_turn>") {
                        eprintln!("✓ Template is using correct Gemma-3 format");
                    }
                }
                ModelFormat::MistralV3 => {
                    if prompt_str.contains("[INST]") {
                        eprintln!("✓ Template is using correct Mistral V3 format");
                    }
                }
                ModelFormat::Qwen3 => {
                    if prompt_str.contains("<|im_start|>") {
                        eprintln!("✓ Template is using correct Qwen3 ChatML format");
                    }
                }
                ModelFormat::GLM4 => {
                    if prompt_str.contains("<|user|>") || prompt_str.contains("[gMASK]") {
                        eprintln!("✓ Template is using correct GLM-4 format");
                    }
                }
            }
        }

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let model = self.model.clone();
        let streaming_config = StreamingConfig {
            temperature: self.config.temperature,
            top_p: self.config.top_p,
            top_k: self.config.top_k,
            max_tokens: self.config.max_tokens,
            seed: self.config.seed,
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
        let model = self.model.clone();
        let mtmd_ctx = self
            .mtmd_ctx
            .clone()
            .ok_or_else(|| Error::Config("Vision context not initialized".to_string()))?;
        let streaming_config = StreamingConfig {
            temperature: self.config.temperature,
            top_p: self.config.top_p,
            top_k: self.config.top_k,
            max_tokens: self.config.max_tokens,
            seed: self.config.seed,
        };

        tokio::spawn(async move {
            generate_tokens_with_vision(model, mtmd_ctx, messages, streaming_config, tx).await;
        });

        Ok(UnboundedReceiverStream::new(rx))
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
                name: self.name.clone(),
                config,
                mtmd_ctx: self.mtmd_ctx.clone(),
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

            McpConverter::to_local_prompt(&tool_infos, self.config.tool_format)
        } else {
            String::new()
        };

        let mut modified_messages = messages.clone();
        if !system_prompt.is_empty() {
            if let Some(first) = modified_messages.first_mut() {
                if first.role == Role::System {
                    first.content = format!("{}\n\n{}", system_prompt, first.content);
                } else {
                    modified_messages.insert(
                        0,
                        Message {
                            role: Role::System,
                            content: system_prompt,
                            images: None,
                        },
                    );
                }
            } else {
                modified_messages.push(Message {
                    role: Role::System,
                    content: system_prompt,
                    images: None,
                });
            }
        }

        let content = self
            .complete_with_options(modified_messages, max_tokens)
            .await?;

        let tool_calls = if tools.is_some() {
            // Try configured format first, then fallback chain
            match self.config.tool_format {
                LocalToolFormat::Fence => ToolParser::parse_fence(&content)
                    .or_else(|_| ToolParser::parse_xml(&content))
                    .unwrap_or_default(),
                LocalToolFormat::Xml => ToolParser::parse_xml(&content)
                    .or_else(|_| ToolParser::parse_fence(&content))
                    .unwrap_or_default(),
                LocalToolFormat::Json => ToolParser::parse_json(&content)
                    .or_else(|_| ToolParser::parse_fence(&content))
                    .unwrap_or_default(),
            }
        } else {
            Vec::new()
        };

        Ok(ProviderResponse {
            content,
            reasoning_content: None,
            tool_calls,
            finish_reason: None,
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
