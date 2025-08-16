use crate::local::config::LocalConfig;
use crate::provider::{Provider as CompletionProvider, StreamResponse};
use crate::{Error, Message, Result, Role};
use async_trait::async_trait;
use candle_core::Tensor;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_stream::Stream;

struct Inner {
    model_loader: super::model_loader::ModelLoader,
    _seed: u64,
    _temperature: f64,
    _top_p: Option<f64>,
}

pub struct CandleProvider {
    inner: Arc<Mutex<Inner>>,
    model_name: String,
}

impl CandleProvider {
    pub fn new(
        model_name: String,
        model_path: Option<String>,
        config: &LocalConfig,
    ) -> Result<Self> {
        let model_loader =
            super::model_loader::ModelLoader::new(&model_name, model_path.as_deref())?;

        Ok(Self {
            inner: Arc::new(Mutex::new(Inner {
                model_loader,
                _seed: config.seed,
                _temperature: config.temperature,
                _top_p: config.top_p,
            })),
            model_name,
        })
    }

    pub async fn initialize(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        guard.model_loader.load_model()
    }
}

#[async_trait]
impl crate::provider::Provider for CandleProvider {
    async fn complete(&self, messages: Vec<Message>) -> Result<String> {
        let prompt = messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        let (mut ids, eos_token_ids, tokenizer, _is_gemma) = {
            let guard = self.inner.lock().await;

            if !guard.model_loader.is_loaded() {
                return Err(Error::Model(
                    "Model not loaded. Call initialize() first.".to_string(),
                ));
            }

            let tokenizer = guard.model_loader.tokenizer()?;
            let is_gemma = super::tokenizer_utils::is_gemma_tokenizer(&tokenizer);
            let formatted_prompt = if is_gemma {
                super::tokenizer_utils::format_gemma_prompt(&prompt, &tokenizer)
            } else {
                prompt.to_string()
            };

            let encoding = tokenizer
                .encode(formatted_prompt.as_str(), true)
                .map_err(|e| Error::Model(format!("Failed to encode prompt: {e}")))?;
            let ids: Vec<u32> = encoding.get_ids().to_vec();

            let eos_ids = if guard.model_loader.eos_token_ids().is_empty() {
                super::tokenizer_utils::get_eos_token_ids(&tokenizer)
            } else {
                guard.model_loader.eos_token_ids().to_vec()
            };

            (ids, eos_ids, tokenizer, is_gemma)
        };

        let context_window = {
            let guard = self.inner.lock().await;
            guard.model_loader.context_length()
        };

        let prompt_len = ids.len();
        let context_remaining = context_window.saturating_sub(prompt_len);
        let max_tokens = 2048.min(context_remaining);

        if max_tokens == 0 {
            return Err(Error::Model(format!(
                "Prompt too long: {prompt_len} tokens exceeds context window of {context_window}"
            )));
        }

        let start_len = ids.len();
        ids.reserve(max_tokens);

        let device = {
            let guard = self.inner.lock().await;
            guard.model_loader.device().clone()
        };

        for index in 0..max_tokens {
            let (input_tokens, position) = if index == 0 {
                (ids.as_slice(), 0)
            } else {
                (&ids[ids.len() - 1..ids.len()], prompt_len + index - 1)
            };

            let next = {
                let mut guard = self.inner.lock().await;
                let mut logits = match guard.model_loader.get_provider_mut() {
                    Some(provider) => {
                        // TODO: This is a hack. We need a better way to do this.
                        let candle_provider =
                            provider.as_any().downcast_ref::<CandleProvider>().unwrap();
                        let mut inner_guard = candle_provider.inner.lock().await;
                        match inner_guard.model_loader.get_model_mut() {
                            Some(super::model_loader::Model::QuantizedLlama(model)) => {
                                let input = Tensor::new(input_tokens, &device)?.unsqueeze(0)?;
                                model.forward(&input, position)?
                            }
                            Some(super::model_loader::Model::QuantizedPhi(model)) => {
                                let input = Tensor::new(input_tokens, &device)?.unsqueeze(0)?;
                                model.forward(&input, position)?
                            }
                            None => {
                                return Err(Error::Model("Model not loaded".to_string()));
                            }
                        }
                    }
                    None => {
                        return Err(Error::Model("Model not loaded".to_string()));
                    }
                };

                if logits.dtype() != candle_core::DType::F32 {
                    logits = logits.to_dtype(candle_core::DType::F32)?;
                }

                let logits_1d = logits.squeeze(0)?;
                let sampling_params = super::sampling::SamplingParams::default();
                super::sampling::sample_next_token(
                    &logits_1d,
                    sampling_params.temperature,
                    sampling_params.top_p,
                    sampling_params.repetition_penalty,
                    &ids[start_len..],
                )?
            };

            ids.push(next);

            if eos_token_ids.contains(&next) {
                break;
            }
        }

        let generated_ids = &ids[start_len..];
        let mut output = tokenizer
            .decode(generated_ids, false)
            .map_err(|e| Error::Model(format!("Failed to decode output: {e}")))?;

        if let Some(stripped) = output.strip_suffix("<|endoftext|>") {
            output = stripped.to_string();
        }

        Ok(output)
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        let result = self.complete(messages).await?;
        let items = vec![Ok(StreamResponse {
            content: result,
            done: true,
        })];
        let stream = tokio_stream::iter(items);
        Ok(Box::new(stream))
    }

    fn name(&self) -> &str {
        &self.model_name
    }
}
