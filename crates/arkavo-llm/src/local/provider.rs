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
struct Inner {
    model_loader: ModelLoader,
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
                let ids: Vec<i64> = encoding.get_ids().iter().map(|&u| u as i64).collect();

                // Get EOS token from metadata or fallback to vocab_size - 1
                let eos_id = guard
                    .model_loader
                    .eos_token_id()
                    .map(|id| id as i64)
                    .unwrap_or_else(|| {
                        let vocab_size = tokenizer.get_vocab_size(false);
                        if vocab_size > 0 {
                            i64::try_from(vocab_size - 1).unwrap_or(i64::MAX)
                        } else {
                            0
                        }
                    });

                (ids, eos_id, tokenizer)
            };

            // Add max_tokens limit with reasonable default
            const MAX_GENERATION_TOKENS: usize = 2048;
            let max_tokens = 60.min(MAX_GENERATION_TOKENS);
            let start_len = ids.len();

            // Reserve capacity for generated tokens
            ids.reserve(max_tokens);

            // Get device from model loader
            let device = {
                let guard = self.inner.lock().await;
                guard.model_loader.device().clone()
            };

            for _ in 0..max_tokens {
                let input = Tensor::new(ids.as_slice(), &device)
                    .map_err(|e| Error::Model(format!("Failed to create tensor: {e}")))?
                    .unsqueeze(0)
                    .map_err(|e| Error::Model(format!("Failed to unsqueeze: {e}")))?;

                // Lock for forward pass
                let next = {
                    let mut guard = self.inner.lock().await;
                    let seq_len = ids.len();

                    // Check model type first
                    let is_quantized =
                        matches!(guard.model_loader.get_model(), Some(Model::Quantized(_)));

                    let logits = if is_quantized {
                        // Simple path for quantized models
                        match guard.model_loader.get_model_mut() {
                            Some(Model::Quantized(w)) => w
                                .forward(&input, seq_len - 1)
                                .map_err(|e| Error::Model(format!("Forward pass failed: {e}")))?,
                            _ => unreachable!(),
                        }
                    } else {
                        // Non-quantized models require proper cache management
                        // TODO: Implement with worker task pattern to avoid borrow checker issues
                        return Err(Error::Model(
                            "Standard Llama models not yet supported - use quantized models (.gguf) for now".to_string()
                        ));
                    };

                    logits
                        .squeeze(0)
                        .map_err(|e| Error::Model(format!("Failed to squeeze: {e}")))?
                        .argmax(0)
                        .map_err(|e| Error::Model(format!("Failed to argmax: {e}")))?
                        .to_scalar::<i64>()
                        .map_err(|e| Error::Model(format!("Failed to get scalar: {e}")))?
                };

                ids.push(next);
                if next == eos_token_id {
                    break;
                }
            }

            // Decode only the generated tokens
            let generated_ids: Vec<u32> = ids[start_len..].iter().map(|&v| v as u32).collect();

            // Use cloned tokenizer for decoding
            let output = tokenizer
                .decode(&generated_ids, true)
                .map_err(|e| Error::Model(format!("Failed to decode output: {e}")))?;

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
            // For now, return a simple stream implementation
            // This will be implemented with actual streaming inference
            let response = self.complete(_messages).await?;

            // Create a vector stream that implements Unpin
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
    }

    fn name(&self) -> &str {
        &self.model_name
    }
}
