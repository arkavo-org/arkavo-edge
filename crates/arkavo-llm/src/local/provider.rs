use crate::{Error, Message, Provider, Result, StreamResponse};
use async_trait::async_trait;
use tokio_stream::Stream;

#[cfg(feature = "local")]
use super::model_loader::{Model, ModelLoader};

#[cfg(feature = "local")]
use candle_core::Tensor;

#[cfg(feature = "local")]
use std::sync::Arc;

#[cfg(feature = "local")]
use tokio::sync::Mutex;

#[cfg(feature = "local")]
use super::worker::WorkerHandle;

#[cfg(feature = "local")]
struct Inner {
    model_loader: ModelLoader,
    #[allow(dead_code)]
    worker_handle: Option<WorkerHandle>,
    _seed: u64,
    _temperature: f64,
    _top_p: Option<f64>,
}

pub struct LocalProvider {
    #[cfg(feature = "local")]
    inner: Arc<Mutex<Inner>>,
    model_name: String,
}

impl LocalProvider {
    pub fn new(model_name: String, model_path: Option<String>) -> Result<Self> {
        #[cfg(not(feature = "local"))]
        {
            return Err(Error::Config(
                "Local provider requires 'local' feature to be enabled".to_string(),
            ));
        }

        #[cfg(feature = "local")]
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

    #[cfg(feature = "local")]
    #[allow(clippy::missing_panics_doc)]
    #[allow(clippy::significant_drop_tightening)]
    pub async fn initialize(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        guard.model_loader.load_model()?;

        // If it's a non-quantized model, create the worker
        if let Some(Model::QuantizedLlama(_model)) = guard.model_loader.get_model() {
            if let Some(_config) = guard.model_loader.get_config() {
                let _device = guard.model_loader.device().clone();
                // We need to move the model out temporarily
                // This is a limitation we'll need to work around
                tracing::warn!(
                    "Non-quantized model detected. Worker pattern not yet fully integrated."
                );
            }
        }

        Ok(())
    }
}

#[async_trait]
impl Provider for LocalProvider {
    #[allow(clippy::significant_drop_tightening)]
    async fn complete(&self, messages: Vec<Message>) -> Result<String> {
        #[cfg(not(feature = "local"))]
        {
            return Err(Error::Config(
                "Local provider requires 'local' feature to be enabled".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let prompt = messages
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");

            // First, get tokenizer and encode the prompt (clone tokenizer for safety)
            let (mut ids, eos_token_id, tokenizer) = {
                let guard = self.inner.lock().await;

                // Check if model is loaded
                if !guard.model_loader.is_loaded() {
                    return Err(Error::Model(
                        "Model not loaded. Call initialize() first.".to_string(),
                    ));
                }

                let tokenizer = guard.model_loader.tokenizer()?;
                let encoding = tokenizer
                    .encode(prompt.as_str(), true)
                    .map_err(|e| Error::Model(format!("Failed to encode prompt: {e}")))?;
                let ids: Vec<u32> = encoding.get_ids().to_vec();

                // Get EOS token from metadata or tokenizer
                let eos_id = guard
                    .model_loader
                    .eos_token_id()
                    .or_else(|| {
                        // Try to get from tokenizer's special tokens
                        super::tokenizer_utils::get_eos_token_id(&tokenizer)
                    })
                    .unwrap_or_else(|| {
                        // Final fallback
                        let vocab_size = tokenizer.get_vocab_size(false);
                        if vocab_size > 0 {
                            (vocab_size - 1) as u32
                        } else {
                            0
                        }
                    });

                (ids, eos_id, tokenizer)
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
                    "Prompt too long: {} tokens exceeds context window of {}",
                    prompt_len,
                    context_window
                )));
            }
            
            let start_len = ids.len();

            let start_time = std::time::Instant::now();

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
                // Lock for forward pass
                let next = {
                    let mut guard = self.inner.lock().await;
                    
                    // On first iteration (index=0), process the entire prompt
                    // On subsequent iterations, only process the last generated token
                    let (input_tokens, position) = if index == 0 {
                        // First pass: process entire prompt
                        (ids.as_slice(), 0)
                    } else {
                        // Subsequent passes: only the last token
                        (&ids[ids.len()-1..ids.len()], prompt_len + index - 1)
                    };

                    // Forward pass based on model architecture
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
                            tracing::debug!("Llama input shape: {:?}, position: {}", input.shape(), position);
                            
                            model
                                .forward(&input, position)
                                .map_err(|e| Error::Model(format!("Llama forward pass failed: {e}")))?
                        }
                        Some(Model::QuantizedPhi(model)) => {
                            // QuantizedPhi expects 2D tensor with shape [1, seq_len]
                            let input = Tensor::new(input_tokens, &device)
                                .map_err(|e| Error::Model(format!("Failed to create tensor: {e}")))?
                                .unsqueeze(0)
                                .map_err(|e| Error::Model(format!("Failed to unsqueeze: {e}")))?;
                            
                            model
                                .forward(&input, position)
                                .map_err(|e| Error::Model(format!("Phi forward pass failed: {e}")))?
                        }
                        None => {
                            return Err(Error::Model("Model not loaded".to_string()));
                        }
                    };

                    // Ensure logits are F32 for subsequent operations
                    if logits.dtype() != candle_core::DType::F32 {
                        logits = logits.to_dtype(candle_core::DType::F32)
                            .map_err(|e| Error::Model(format!("Failed to convert logits to F32: {e}")))?;
                    }

                    logits
                        .squeeze(0)
                        .map_err(|e| Error::Model(format!("Failed to squeeze: {e}")))?
                        .argmax(0)
                        .map_err(|e| Error::Model(format!("Failed to argmax: {e}")))?
                        .to_scalar::<u32>()
                        .map_err(|e| Error::Model(format!("Failed to get scalar: {e}")))?
                };

                ids.push(next);
                
                // Early stop on EOS token
                if next == eos_token_id {
                    break;
                }
            }

            // Decode the full sequence to get complete output
            let output = tokenizer
                .decode(&ids, true)
                .map_err(|e| Error::Model(format!("Failed to decode output: {e}")))?;

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
        #[cfg(not(feature = "local"))]
        {
            return Err(Error::Config(
                "Local provider requires 'local' feature to be enabled".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let prompt = _messages
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");

            // Lock once to check model type and get necessary data
            let guard = self.inner.lock().await;

            // Check if model is loaded
            if !guard.model_loader.is_loaded() {
                return Err(Error::Model(
                    "Model not loaded. Call initialize() first.".to_string(),
                ));
            }

            let tokenizer = guard.model_loader.tokenizer()?;
            let encoding = tokenizer
                .encode(prompt.as_str(), true)
                .map_err(|e| Error::Model(format!("Failed to encode prompt: {e}")))?;
            let _ids: Vec<u32> = encoding.get_ids().to_vec();

            // Get EOS token
            let _eos_id = guard
                .model_loader
                .eos_token_id()
                .or_else(|| super::tokenizer_utils::get_eos_token_id(&tokenizer))
                .unwrap_or(0);

            // Check if we have a model loaded
            let has_model = guard.model_loader.get_model().is_some();

            drop(guard); // Release lock before streaming

            if has_model {
                // Fall back to non-streaming for now
                let response = self.complete(_messages).await?;
                let items = vec![Ok(StreamResponse {
                    content: response,
                    done: true,
                })];
                let stream = tokio_stream::iter(items);
                Ok(Box::new(stream)
                    as Box<
                        dyn Stream<Item = Result<StreamResponse>> + Send + Unpin,
                    >)
            } else {
                // For non-quantized models, we would use the streaming worker
                // but we need to properly handle model ownership first
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
