#[cfg(feature = "llm-local")]
use super::model_loader::Model;
#[cfg(feature = "llm-local")]
use crate::{Error, Result};
#[cfg(feature = "llm-local")]
use candle_core::Tensor;
#[cfg(feature = "llm-local")]
use std::sync::Arc;
#[cfg(feature = "llm-local")]
use tokenizers::Tokenizer;
#[cfg(feature = "llm-local")]
use tokio::sync::Mutex;

#[cfg(feature = "llm-local")]
#[allow(clippy::significant_drop_tightening)]
pub async fn generate_tokens(
    inner: Arc<Mutex<super::provider::Inner>>,
    model_name: &str,
    mut ids: Vec<u32>,
    eos_token_ids: Vec<u32>,
    tokenizer: Tokenizer,
    max_tokens: usize,
) -> Result<String> {
    let start_len = ids.len();
    let start_time = std::time::Instant::now();

    tracing::info!(
        "Starting generation with {} prompt tokens, max {} tokens to generate",
        ids.len(),
        max_tokens
    );

    ids.reserve(max_tokens);

    let device = {
        let guard = inner.lock().await;
        let device = guard.model_loader.device().clone();
        drop(guard);
        device
    };

    let prompt_len = ids.len();

    for index in 0..max_tokens {
        let token_start = std::time::Instant::now();

        let next = {
            let mut guard = inner.lock().await;

            if index % 10 == 0 || index < 3 {
                tracing::info!(
                    "[generation] Step {}/{}, total time: {:?}",
                    index,
                    max_tokens,
                    start_time.elapsed()
                );
            }

            let (input_tokens, position) = if index == 0 {
                tracing::debug!("[generation] Processing full prompt: {} tokens", ids.len());
                (ids.as_slice(), 0)
            } else {
                (&ids[ids.len() - 1..ids.len()], prompt_len + index - 1)
            };

            tracing::debug!("[generation] Running forward pass at position {}", position);

            let mut logits = match guard.model_loader.get_model_mut() {
                Some(Model::QuantizedGemma3(model)) => {
                    let input = Tensor::new(input_tokens, &device)
                        .map_err(|e| Error::Model(format!("Failed to create tensor: {e}")))?
                        .unsqueeze(0)
                        .map_err(|e| Error::Model(format!("Failed to unsqueeze: {e}")))?;
                    model
                        .forward(&input, position)
                        .map_err(|e| Error::Model(format!("Gemma3 forward pass failed: {e}")))?
                }
                Some(Model::QuantizedLlama(model)) => {
                    let input = Tensor::new(input_tokens, &device)
                        .map_err(|e| Error::Model(format!("Failed to create tensor: {e}")))?
                        .unsqueeze(0)
                        .map_err(|e| Error::Model(format!("Failed to unsqueeze: {e}")))?;

                    tracing::debug!(
                        "Llama input shape: {:?}, position: {}",
                        input.shape(),
                        position
                    );

                    model
                        .forward(&input, position)
                        .map_err(|e| Error::Model(format!("Llama forward pass failed: {e}")))?
                }
                Some(Model::QuantizedPhi(model)) => {
                    let input = Tensor::new(input_tokens, &device)
                        .map_err(|e| Error::Model(format!("Failed to create tensor: {e}")))?
                        .unsqueeze(0)
                        .map_err(|e| Error::Model(format!("Failed to unsqueeze: {e}")))?;

                    model
                        .forward(&input, position)
                        .map_err(|e| Error::Model(format!("Phi forward pass failed: {e}")))?
                }
                #[cfg(all(target_os = "linux", target_arch = "aarch64", feature = "snpe"))]
                Some(Model::Snpe(_)) => {
                    return Err(Error::Inference(
                        "SNPE models use async execution, not forward pass".to_string(),
                    ));
                }
                None => {
                    return Err(Error::Model("Model not loaded".to_string()));
                }
            };

            if logits.dtype() != candle_core::DType::F32 {
                logits = logits
                    .to_dtype(candle_core::DType::F32)
                    .map_err(|e| Error::Model(format!("Failed to convert logits to F32: {e}")))?;
            }

            let logits_1d = logits
                .squeeze(0)
                .map_err(|e| Error::Model(format!("Failed to squeeze: {e}")))?;

            let sampling_params = super::sampling::SamplingParams::default();
            super::sampling::sample_next_token(
                &logits_1d,
                sampling_params.temperature,
                sampling_params.top_p,
                sampling_params.repetition_penalty,
                &ids[start_len..],
            )?
        };

        let token_time = token_start.elapsed();
        if index < 5 || index % 20 == 0 {
            tracing::info!(
                "[generation] Token {} generated: id={}, time={:?}",
                index,
                next,
                token_time
            );
        }

        ids.push(next);

        if eos_token_ids.contains(&next) {
            tracing::info!("[generation] Hit EOS token {} at position {}", next, index);
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

    let generation_time = start_time.elapsed();
    let tokens_generated = ids.len() - start_len;
    let device_name = {
        let guard = inner.lock().await;
        let name = format!("{:?}", guard.model_loader.device());
        drop(guard);
        name
    };

    tracing::info!(
        model = %model_name,
        device = %device_name,
        prompt_tokens = %start_len,
        generated_tokens = %tokens_generated,
        generation_ms = %generation_time.as_millis(),
        tokens_per_second = %(tokens_generated as f64 / generation_time.as_secs_f64()),
        "Local generation completed"
    );

    Ok(output)
}
