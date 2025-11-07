use crate::tool_parser::ToolParser;
use crate::{Error, Message, Provider, ProviderResponse, Result, Role, StreamResponse};
use async_trait::async_trait;
use serde_json::Value;
use tokio_stream::Stream;

#[cfg(feature = "llm-local")]
use super::model_loader::{Model, ModelLoader};

#[cfg(feature = "llm-local")]
use std::sync::Arc;

#[cfg(feature = "llm-local")]
use tokio::sync::Mutex;

#[cfg(feature = "llm-local")]
use super::worker::WorkerHandle;

#[cfg(feature = "llm-local")]
pub struct Inner {
    pub model_loader: ModelLoader,
    #[allow(dead_code)]
    worker_handle: Option<WorkerHandle>,
    _seed: u64,
    _temperature: f64,
    _top_p: Option<f64>,
}

pub struct LocalProvider {
    #[cfg(feature = "llm-local")]
    inner: Arc<Mutex<Inner>>,
    model_name: String,
}

impl LocalProvider {
    /// Resolve a HuggingFace repo ID to an actual .gguf file path in the cache
    fn resolve_hf_repo_to_path(repo_id: &str) -> Option<String> {
        // Convert repo ID like "unsloth/gemma-3-270m-it-GGUF" to cache directory name
        // "models--unsloth--gemma-3-270m-it-GGUF"
        let cache_dir_name = format!("models--{}", repo_id.replace('/', "--"));

        // Get HuggingFace cache directory
        let home = dirs::home_dir()?;
        let cache_base = home.join(".cache/huggingface/hub");
        let repo_path = cache_base.join(&cache_dir_name);

        if !repo_path.exists() {
            tracing::warn!(
                "HuggingFace cache directory not found: {}",
                repo_path.display()
            );
            return None;
        }

        // Look in snapshots directory
        let snapshots_dir = repo_path.join("snapshots");
        if let Ok(entries) = std::fs::read_dir(&snapshots_dir) {
            for entry in entries.flatten() {
                let snapshot_path = entry.path();
                // Find first .gguf file in this snapshot
                if let Ok(files) = std::fs::read_dir(&snapshot_path) {
                    for file in files.flatten() {
                        let file_path = file.path();
                        if file_path.extension().and_then(|s| s.to_str()) == Some("gguf") {
                            let path_str = file_path.to_string_lossy().to_string();
                            tracing::info!(
                                "Resolved repo ID '{}' to model file: {}",
                                repo_id,
                                path_str
                            );
                            return Some(path_str);
                        }
                    }
                }
            }
        }

        tracing::warn!("No .gguf file found for repo ID: {}", repo_id);
        None
    }

    pub fn new(model_name: String, model_path: Option<String>) -> Result<Self> {
        #[cfg(not(feature = "llm-local"))]
        {
            return Err(Error::Config(
                "Local provider requires 'llm-local' feature to be enabled".to_string(),
            ));
        }

        #[cfg(feature = "llm-local")]
        {
            // Resolve model path - if it contains '/', it's likely a repo ID
            let resolved_path = model_path
                .as_ref()
                .and_then(|path| {
                    if path.contains('/') && !path.starts_with('/') {
                        // Looks like a repo ID (e.g., "unsloth/gemma-3-270m-it-GGUF")
                        Self::resolve_hf_repo_to_path(path)
                    } else {
                        // It's already a file path
                        Some(path.clone())
                    }
                })
                .or(model_path);

            let model_loader = ModelLoader::new(&model_name, resolved_path.as_deref())?;

            Ok(Self {
                inner: Arc::new(Mutex::new(Inner {
                    model_loader,
                    worker_handle: None,
                    _seed: 42,
                    _temperature: 0.8,
                    _top_p: Some(0.95),
                })),
                model_name,
            })
        }
    }

    #[cfg(feature = "llm-local")]
    #[allow(clippy::missing_panics_doc)]
    #[allow(clippy::significant_drop_tightening)]
    pub async fn initialize(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        guard.model_loader.load_model()?;

        // If it's a non-quantized model, create the worker
        if let Some(Model::QuantizedLlama(_model)) = guard.model_loader.get_model()
            && let Some(_config) = guard.model_loader.get_config()
        {
            let _device = guard.model_loader.device().clone();
            // We need to move the model out temporarily
            // This is a limitation we'll need to work around
            tracing::warn!(
                "Non-quantized model detected. Worker pattern not yet fully integrated."
            );
        }

        Ok(())
    }
}

#[async_trait]
impl Provider for LocalProvider {
    #[allow(clippy::significant_drop_tightening)]
    async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        max_tokens: Option<usize>,
    ) -> Result<String> {
        #[cfg(not(feature = "llm-local"))]
        {
            return Err(Error::Config(
                "Local provider requires 'llm-local' feature to be enabled".to_string(),
            ));
        }

        #[cfg(feature = "llm-local")]
        {
            tracing::info!(
                "[LocalProvider::complete] Starting completion for {} messages",
                messages.len()
            );

            let prompt = messages
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");

            tracing::debug!(
                "[LocalProvider::complete] Combined prompt: {} chars",
                prompt.len()
            );

            // First, get tokenizer and encode the prompt (clone tokenizer for safety)
            let (ids, eos_token_ids, tokenizer, _is_gemma) = {
                tracing::debug!("[LocalProvider::complete] Acquiring model lock...");
                let guard = self.inner.lock().await;
                tracing::debug!("[LocalProvider::complete] Model lock acquired");

                // Check if model is loaded
                if !guard.model_loader.is_loaded() {
                    tracing::error!("[LocalProvider::complete] Model not loaded!");
                    return Err(Error::Model(
                        "Model not loaded. Call initialize() first.".to_string(),
                    ));
                }

                let tokenizer = guard.model_loader.tokenizer()?;

                // Check if this is a Gemma model and format prompt accordingly
                let is_gemma = super::tokenizer_utils::is_gemma_tokenizer(&tokenizer);
                let formatted_prompt = if is_gemma {
                    super::tokenizer_utils::format_gemma_prompt(&prompt, &tokenizer)
                } else {
                    prompt.clone()
                };

                // Debug log the formatted prompt
                tracing::debug!(
                    "[LocalProvider::complete] Formatted prompt: {:?}",
                    formatted_prompt
                );

                let encoding = tokenizer
                    .encode(formatted_prompt.as_str(), true)
                    .map_err(|e| Error::Model(format!("Failed to encode prompt: {e}")))?;
                let ids: Vec<u32> = encoding.get_ids().to_vec();

                // Get EOS tokens from model loader or tokenizer
                let eos_ids = if guard.model_loader.eos_token_ids().is_empty() {
                    super::tokenizer_utils::get_eos_token_ids(&tokenizer)
                } else {
                    guard.model_loader.eos_token_ids().to_vec()
                };

                // Debug log tokenizer info
                tracing::info!(
                    "[LocalProvider::complete] Model: {}, Is Gemma: {}, Prompt tokens: {}, EOS tokens: {:?}",
                    self.model_name,
                    is_gemma,
                    ids.len(),
                    eos_ids
                );
                tracing::debug!("Is Gemma model: {}", is_gemma);
                tracing::debug!("EOS token IDs: {:?}", eos_ids);
                tracing::debug!("Prompt token count: {}", ids.len());

                // Debug check specific Gemma tokens
                if is_gemma {
                    let start_turn_id = tokenizer.token_to_id("<start_of_turn>");
                    let end_turn_id = tokenizer.token_to_id("<end_of_turn>");
                    let model_id = tokenizer.token_to_id("model");
                    tracing::debug!("<start_of_turn> token ID: {:?}", start_turn_id);
                    tracing::debug!("<end_of_turn> token ID: {:?}", end_turn_id);
                    tracing::debug!("'model' token ID: {:?}", model_id);
                }

                (ids, eos_ids, tokenizer, is_gemma)
            };

            // Get context window from model metadata
            let context_window = {
                let guard = self.inner.lock().await;
                guard.model_loader.context_length()
            };

            // Calculate how many tokens we can generate
            let prompt_len = ids.len();
            let context_remaining = context_window.saturating_sub(prompt_len);

            // Use provided max_tokens or default to 2048, capped by remaining context
            let max_tokens = max_tokens.unwrap_or(2048).min(context_remaining);

            tracing::debug!(
                "Context window: {}, prompt length: {}, max tokens to generate: {}",
                context_window,
                prompt_len,
                max_tokens
            );

            if max_tokens == 0 {
                return Err(Error::Model(format!(
                    "Prompt too long: {prompt_len} tokens exceeds context window of {context_window}"
                )));
            }

            super::generation::generate_tokens(
                self.inner.clone(),
                &self.model_name,
                ids,
                eos_token_ids,
                (*tokenizer).clone(),
                max_tokens,
            )
            .await
        }
    }

    async fn stream(
        &self,
        _messages: Vec<Message>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        #[cfg(not(feature = "llm-local"))]
        {
            return Err(Error::Config(
                "Local provider requires 'llm-local' feature to be enabled".to_string(),
            ));
        }

        #[cfg(feature = "llm-local")]
        {
            use std::time::Instant;

            let stream_start = Instant::now();
            tracing::info!(
                "[LocalProvider::stream] Starting stream for {} messages",
                _messages.len()
            );

            let prompt = _messages
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");

            tracing::debug!(
                "[LocalProvider::stream] Prompt length: {} chars",
                prompt.len()
            );

            // Lock once to check model type and get necessary data
            let guard = self.inner.lock().await;

            // Check if model is loaded
            if !guard.model_loader.is_loaded() {
                tracing::error!("[LocalProvider::stream] Model not loaded!");
                return Err(Error::Model(
                    "Model not loaded. Call initialize() first.".to_string(),
                ));
            }

            let tokenizer = guard.model_loader.tokenizer()?;
            let encoding = tokenizer
                .encode(prompt.as_str(), true)
                .map_err(|e| Error::Model(format!("Failed to encode prompt: {e}")))?;
            let _ids: Vec<u32> = encoding.get_ids().to_vec();

            tracing::debug!(
                "[LocalProvider::stream] Encoded prompt to {} tokens",
                _ids.len()
            );

            // Get EOS token
            let _eos_id = guard
                .model_loader
                .eos_token_id()
                .or_else(|| super::tokenizer_utils::get_eos_token_id(&tokenizer))
                .unwrap_or(0);

            // Check if we have a model loaded
            let has_model = guard.model_loader.get_model().is_some();
            let model_name = self.model_name.clone();

            drop(guard); // Release lock before streaming

            if has_model {
                tracing::info!(
                    "[LocalProvider::stream] Model {} loaded, falling back to complete() for streaming",
                    model_name
                );

                // Fall back to non-streaming for now - but with timeout protection
                match tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    self.complete(_messages),
                )
                .await
                {
                    Ok(Ok(response)) => {
                        tracing::info!(
                            "[LocalProvider::stream] Complete() returned {} chars after {:?}",
                            response.len(),
                            stream_start.elapsed()
                        );

                        let items = vec![Ok(StreamResponse {
                            content: response,
                            done: true,
                        })];
                        let stream = tokio_stream::iter(items);
                        Ok(Box::new(stream)
                            as Box<
                                dyn Stream<Item = Result<StreamResponse>> + Send + Unpin,
                            >)
                    }
                    Ok(Err(e)) => {
                        tracing::error!("[LocalProvider::stream] Complete() failed: {}", e);
                        Err(e)
                    }
                    Err(_) => {
                        tracing::error!("[LocalProvider::stream] Complete() timed out after 30s");
                        Err(Error::Model(format!(
                            "Model {model_name} timed out generating response after 30 seconds"
                        )))
                    }
                }
            } else {
                // For non-quantized models, we would use the streaming worker
                // but we need to properly handle model ownership first
                tracing::warn!("[LocalProvider::stream] No model loaded for streaming");
                return Err(Error::Model(
                    "Streaming for non-quantized models not yet implemented".to_string(),
                ));
            }
        }
    }

    fn name(&self) -> &str {
        &self.model_name
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
        #[cfg(not(feature = "llm-local"))]
        {
            return Err(Error::Config(
                "Local provider requires 'llm-local' feature to be enabled".to_string(),
            ));
        }

        #[cfg(feature = "llm-local")]
        {
            use crate::mcp_converter::McpConverter;

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
}
