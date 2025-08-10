use crate::{Error, Message, Provider, Result, StreamResponse};
use async_trait::async_trait;
use tokio_stream::Stream;

#[cfg(feature = "llm-local")]
use super::model_loader::{Model, ModelLoader};

#[cfg(feature = "llm-local")]
use candle_core::Tensor;

#[cfg(feature = "llm-local")]
use std::sync::Arc;

#[cfg(feature = "llm-local")]
use tokio::sync::Mutex;

#[cfg(feature = "llm-local")]
use super::worker::WorkerHandle;

#[cfg(feature = "llm-local")]
struct Inner {
    model_loader: ModelLoader,
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
    pub fn new(model_name: String, model_path: Option<String>) -> Result<Self> {
        #[cfg(not(feature = "llm-local"))]
        {
            return Err(Error::Config(
                "Local provider requires 'llm-local' feature to be enabled".to_string(),
            ));
        }

        #[cfg(feature = "llm-local")]
        {
            let model_loader = ModelLoader::new(&model_name, model_path.as_deref())?;

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
    async fn complete(&self, messages: Vec<Message>) -> Result<String> {
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
            let (mut ids, eos_token_ids, tokenizer, _is_gemma) = {
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

            // Use full remaining context window (up to 2048 tokens)
            let max_tokens = 2048.min(context_remaining);

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

            let start_len = ids.len();

            let start_time = std::time::Instant::now();

            tracing::info!(
                "Starting generation with {} prompt tokens, max {} tokens to generate",
                prompt_len,
                max_tokens
            );

            // Reserve capacity for generated tokens
            ids.reserve(max_tokens);

            // Get device from model loader
            let device = {
                let guard = self.inner.lock().await;
                guard.model_loader.device().clone()
            };

            // Process the prompt first (all tokens at once)
            let prompt_len = ids.len();

            for index in 0..max_tokens {
                let token_start = std::time::Instant::now();

                // Lock for forward pass
                let next = {
                    let mut guard = self.inner.lock().await;

                    if index % 10 == 0 || index < 3 {
                        tracing::info!(
                            "[LocalProvider::complete] Generation step {}/{}, total time: {:?}",
                            index,
                            max_tokens,
                            start_time.elapsed()
                        );
                    }

                    // On first iteration (index=0), process the entire prompt
                    // On subsequent iterations, only process the last generated token
                    let (input_tokens, position) = if index == 0 {
                        // First pass: process entire prompt
                        tracing::debug!(
                            "[LocalProvider::complete] Processing full prompt: {} tokens",
                            ids.len()
                        );
                        (ids.as_slice(), 0)
                    } else {
                        // Subsequent passes: only the last token
                        (&ids[ids.len() - 1..ids.len()], prompt_len + index - 1)
                    };

                    // Forward pass based on model architecture
                    tracing::debug!(
                        "[LocalProvider::complete] Running forward pass at position {}",
                        position
                    );

                    let mut logits = match guard.model_loader.get_model_mut() {
                        Some(Model::QuantizedGemma3(model)) => {
                            // Gemma3 expects batch dimension
                            let input = Tensor::new(input_tokens, &device)
                                .map_err(|e| Error::Model(format!("Failed to create tensor: {e}")))?
                                .unsqueeze(0)
                                .map_err(|e| Error::Model(format!("Failed to unsqueeze: {e}")))?;
                            model.forward(&input, position).map_err(|e| {
                                Error::Model(format!("Gemma3 forward pass failed: {e}"))
                            })?
                        }
                        Some(Model::QuantizedLlama(model)) => {
                            // QuantizedLlama expects 2D tensor with shape [1, seq_len]
                            let input = Tensor::new(input_tokens, &device)
                                .map_err(|e| Error::Model(format!("Failed to create tensor: {e}")))?
                                .unsqueeze(0)
                                .map_err(|e| Error::Model(format!("Failed to unsqueeze: {e}")))?;

                            // Debug print input shape
                            tracing::debug!(
                                "Llama input shape: {:?}, position: {}",
                                input.shape(),
                                position
                            );

                            model.forward(&input, position).map_err(|e| {
                                Error::Model(format!("Llama forward pass failed: {e}"))
                            })?
                        }
                        Some(Model::QuantizedPhi(model)) => {
                            // QuantizedPhi expects 2D tensor with shape [1, seq_len]
                            let input = Tensor::new(input_tokens, &device)
                                .map_err(|e| Error::Model(format!("Failed to create tensor: {e}")))?
                                .unsqueeze(0)
                                .map_err(|e| Error::Model(format!("Failed to unsqueeze: {e}")))?;

                            model.forward(&input, position).map_err(|e| {
                                Error::Model(format!("Phi forward pass failed: {e}"))
                            })?
                        }
                        None => {
                            return Err(Error::Model("Model not loaded".to_string()));
                        }
                    };

                    // Ensure logits are F32 for subsequent operations
                    if logits.dtype() != candle_core::DType::F32 {
                        logits = logits.to_dtype(candle_core::DType::F32).map_err(|e| {
                            Error::Model(format!("Failed to convert logits to F32: {e}"))
                        })?;
                    }

                    // Apply sampling with temperature and repetition penalty
                    let logits_1d = logits
                        .squeeze(0)
                        .map_err(|e| Error::Model(format!("Failed to squeeze: {e}")))?;

                    // Use sampling parameters
                    let sampling_params = super::sampling::SamplingParams::default();
                    super::sampling::sample_next_token(
                        &logits_1d,
                        sampling_params.temperature,
                        sampling_params.top_p,
                        sampling_params.repetition_penalty,
                        &ids[start_len..], // Only apply repetition penalty to generated tokens
                    )?
                };

                let token_time = token_start.elapsed();
                if index < 5 || index % 20 == 0 {
                    tracing::info!(
                        "[LocalProvider::complete] Token {} generated: id={}, time={:?}",
                        index,
                        next,
                        token_time
                    );
                }

                ids.push(next);

                // Early stop on any EOS token
                if eos_token_ids.contains(&next) {
                    tracing::info!(
                        "[LocalProvider::complete] Hit EOS token {} at position {}",
                        next,
                        index
                    );
                    break;
                }
            }

            // Decode only the generated tokens (not the prompt)
            let generated_ids = &ids[start_len..];
            let mut output = tokenizer
                .decode(generated_ids, false)
                .map_err(|e| Error::Model(format!("Failed to decode output: {e}")))?;

            // Manually remove the EOS token if it's present at the end
            if let Some(stripped) = output.strip_suffix("<|endoftext|>") {
                output = stripped.to_string();
            }

            // Log generation metrics
            let generation_time = start_time.elapsed();
            let tokens_generated = ids.len() - start_len;
            let device_name = {
                let guard = self.inner.lock().await;
                format!("{:?}", guard.model_loader.device())
            };

            tracing::info!(
                model = %self.model_name,
                device = %device_name,
                prompt_tokens = %start_len,
                generated_tokens = %tokens_generated,
                generation_ms = %generation_time.as_millis(),
                tokens_per_second = %(tokens_generated as f64 / generation_time.as_secs_f64()),
                "Local generation completed"
            );

            Ok(output)
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
}
