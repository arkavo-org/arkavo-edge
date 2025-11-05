use crate::tool_parser::{ParsedToolCall, ToolParser};
use crate::{Error, Message, Provider, ProviderResponse, Result, Role, StreamResponse};
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llama_cpp::multimodal::MtmdContext;
use arkavo_llama_cpp::{
    LlamaModel, apply_chat_template, ffi, init_llama_logging, test_minimal_init,
};
use async_trait::async_trait;
use serde_json::Value;
use std::ffi::CString;
use std::sync::Arc;
use tokio_stream::{Stream, wrappers::UnboundedReceiverStream};

use crate::llamacpp_streaming::{StreamingConfig, generate_tokens};
use crate::mcp_converter::McpConverter;

#[derive(Debug, Clone)]
pub struct SamplingConfig {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: i32,
    pub max_tokens: u32,
    pub seed: u32,
    pub debug: bool,
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
        }
    }
}

pub struct LlamaCppProvider {
    model: Arc<LlamaModel>,
    name: String,
    config: SamplingConfig,
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    mtmd_ctx: Option<Arc<MtmdContext>>,
}

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

        #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
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
            #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
            mtmd_ctx,
        })
    }

    fn generate_streaming(
        &self,
        messages: Vec<Message>,
    ) -> Result<UnboundedReceiverStream<Result<StreamResponse>>> {
        #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
        let has_images = messages
            .iter()
            .any(|m| m.images.is_some() && !m.images.as_ref().unwrap().is_empty());

        #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
        if has_images && self.mtmd_ctx.is_some() {
            return self.generate_streaming_with_vision(messages);
        }

        let (llama_messages, _cstrings) = Self::messages_to_llama_chat_static(&messages)?;

        let prompt_bytes = apply_chat_template(&llama_messages, true)
            .map_err(|e| Error::Config(format!("Failed to apply chat template: {e}")))?;

        if crate::llamacpp_streaming::is_debug()
            && let Ok(prompt_str) = std::str::from_utf8(&prompt_bytes)
        {
            eprintln!("Chat template output:\n{prompt_str}");
            if prompt_str.contains("<|im_start|>") {
                eprintln!("WARNING: Template is using Llama-3 format, not Gemma-3!");
            } else if prompt_str.contains("<start_of_turn>") {
                eprintln!("✓ Template is using correct Gemma-3 format");
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

    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
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
                #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
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

            let tool_infos: Vec<arkavo_mcp_tools::ToolInfo> = tools_array
                .iter()
                .filter_map(|t| {
                    Some(arkavo_mcp_tools::ToolInfo {
                        name: t.get("name")?.as_str()?.to_string(),
                        description: t.get("description")?.as_str()?.to_string(),
                        schema: t.get("input_schema")?.clone(),
                    })
                })
                .collect();

            McpConverter::to_xml_prompt(&tool_infos)
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
            ToolParser::parse_xml(&content).unwrap_or_default()
        } else {
            Vec::new()
        };

        Ok(ProviderResponse {
            content,
            tool_calls,
            finish_reason: None,
        })
    }
}
