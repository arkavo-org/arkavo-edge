#![allow(clippy::redundant_pub_crate)]

use crate::{Error, Message, Result, StreamResponse, decode_image};
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llama_cpp::multimodal::{
    MtmdBitmap, MtmdContext, default_media_marker, encode_chunk, get_output_embeddings,
    preprocess_image_for_clip, tokenize_with_images,
};
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llama_cpp::{
    LlamaContext, LlamaModel, batch_free, batch_get_one_with_logits, batch_get_one_with_offset,
    batch_init_with_tokens, create_sampler_chain, decode_batch, token_to_bytes,
    tokenize_with_model,
};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;

static DEBUG_LLAMACPP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn set_debug(enabled: bool) {
    DEBUG_LLAMACPP.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn is_debug() -> bool {
    DEBUG_LLAMACPP.load(std::sync::atomic::Ordering::Relaxed)
}

/// Extract valid UTF-8 from a byte buffer, leaving incomplete sequences for later.
/// Returns the valid string and modifies the buffer in place to keep only incomplete bytes.
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
fn extract_valid_utf8(buffer: &mut Vec<u8>) -> String {
    if buffer.is_empty() {
        return String::new();
    }

    // Find the longest valid UTF-8 prefix
    match std::str::from_utf8(buffer) {
        Ok(s) => {
            // Entire buffer is valid UTF-8
            let result = s.to_string();
            buffer.clear();
            result
        }
        Err(e) => {
            let valid_up_to = e.valid_up_to();
            if valid_up_to == 0 {
                // Check if we're in the middle of a multi-byte sequence
                // If the buffer starts with a continuation byte or incomplete sequence,
                // we need to wait for more bytes
                if buffer.len() < 4 && is_incomplete_utf8_start(buffer) {
                    return String::new();
                }
                // Otherwise, skip the invalid byte (shouldn't happen with proper tokenizers)
                buffer.remove(0);
                String::new()
            } else {
                // Extract the valid portion
                let valid_bytes: Vec<u8> = buffer.drain(..valid_up_to).collect();
                // SAFETY: We know these bytes are valid UTF-8 from the error
                unsafe { String::from_utf8_unchecked(valid_bytes) }
            }
        }
    }
}

/// Check if the buffer contains the start of an incomplete multi-byte UTF-8 sequence
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
fn is_incomplete_utf8_start(buffer: &[u8]) -> bool {
    if buffer.is_empty() {
        return false;
    }
    let first = buffer[0];
    // Check for multi-byte sequence starters
    if first & 0x80 == 0 {
        // ASCII - should be valid
        false
    } else if first & 0xE0 == 0xC0 {
        // 2-byte sequence (110xxxxx) - need 2 bytes
        buffer.len() < 2
    } else if first & 0xF0 == 0xE0 {
        // 3-byte sequence (1110xxxx) - need 3 bytes
        buffer.len() < 3
    } else if first & 0xF8 == 0xF0 {
        // 4-byte sequence (11110xxx) - need 4 bytes
        buffer.len() < 4
    } else {
        // Continuation byte or invalid - not an incomplete start
        false
    }
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[derive(Debug, Clone)]
pub(crate) struct StreamingConfig {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: i32,
    pub max_tokens: u32,
    pub seed: u32,
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub(crate) async fn generate_tokens(
    model: Arc<LlamaModel>,
    prompt_bytes: Vec<u8>,
    config: StreamingConfig,
    tx: UnboundedSender<Result<StreamResponse>>,
) {
    let start_time = Instant::now();
    let mut first_token_time: Option<Instant> = None;
    let mut tokens_generated = 0u32;

    #[allow(clippy::significant_drop_tightening)]
    let result = async {
        let ctx = LlamaContext::new(&model)
            .map_err(|e| Error::Config(format!("Failed to create context: {e}")))?;

        let vocab = model.get_vocab();
        let input_tokens = tokenize_with_model(vocab, &prompt_bytes)
            .map_err(|e| Error::Config(format!("Failed to tokenize: {e}")))?;

        tracing::info!("Input tokenized to {} tokens", input_tokens.len());
        if is_debug() {
            eprintln!(
                "🔍 First 10 tokens: {:?}",
                input_tokens.iter().take(10).collect::<Vec<_>>()
            );
        }

        let sampler =
            create_sampler_chain(config.temperature, config.top_p, config.top_k, config.seed)
                .map_err(|e| Error::Config(format!("Failed to create sampler: {e}")))?;

        let eos_token = model.get_eos_token();
        if is_debug() {
            eprintln!("EOS token from model: {eos_token}");
        }

        process_input_tokens(&ctx, &input_tokens)?;

        let max_generation = std::cmp::min(config.max_tokens, 30000);
        let mut pos = i32::try_from(input_tokens.len()).unwrap_or(0);

        if is_debug() {
            eprintln!(
                "🎯 Max generation tokens: {} (config.max_tokens: {})",
                max_generation, config.max_tokens
            );
        }

        // Buffer for incomplete UTF-8 sequences
        let mut utf8_buffer: Vec<u8> = Vec::new();

        for _ in 0..max_generation {
            validate_logits(&ctx)?;

            let token = sampler.sample(&ctx, -1);
            if is_debug() {
                eprintln!("Sampled token: {token} at pos {pos}");
            }

            if token == eos_token {
                tracing::info!("Generation stopped at EOS token");
                // Flush any remaining buffer as lossy UTF-8
                if !utf8_buffer.is_empty() {
                    let piece = String::from_utf8_lossy(&utf8_buffer).to_string();
                    let _ = tx.send(Ok(StreamResponse {
                        content: piece,
                        reasoning_content: None,
                        done: false,
                    }));
                }
                break;
            }

            // Get raw bytes for this token
            let token_bytes = token_to_bytes(vocab, token, false)
                .map_err(|e| Error::Config(format!("Failed to decode token: {e}")))?;

            // Add to buffer
            utf8_buffer.extend_from_slice(&token_bytes);

            // Try to extract valid UTF-8 from the buffer
            let piece = extract_valid_utf8(&mut utf8_buffer);

            if first_token_time.is_none() && !piece.is_empty() {
                first_token_time = Some(Instant::now());
            }

            tokens_generated += 1;

            // Only send if we have content
            if !piece.is_empty() {
                let response = StreamResponse {
                    content: piece,
                    reasoning_content: None,
                    done: false,
                };

                if tx.send(Ok(response)).is_err() {
                    break;
                }
            }

            sampler.accept(token);

            let mut batch = batch_init_with_tokens(&[token], pos, true);
            decode_batch(&ctx, batch)
                .map_err(|e| Error::Config(format!("Failed to decode token at pos {pos}: {e}")))?;
            batch_free(&mut batch);

            pos += 1;

            let total_tokens = input_tokens.len() + tokens_generated as usize;
            if total_tokens >= 32000 {
                tracing::info!(
                    "Generation stopped at context limit: {} tokens",
                    total_tokens
                );
                // Flush remaining buffer
                if !utf8_buffer.is_empty() {
                    let piece = String::from_utf8_lossy(&utf8_buffer).to_string();
                    let _ = tx.send(Ok(StreamResponse {
                        content: piece,
                        reasoning_content: None,
                        done: false,
                    }));
                }
                let _ = tx.send(Ok(StreamResponse {
                    content: String::new(),
                    reasoning_content: None,
                    done: true,
                }));
                break;
            }
        }

        Ok::<(u32, Option<Instant>), Error>((tokens_generated, first_token_time))
    }
    .await;

    match result {
        Ok((tokens_generated, first_token_time)) => {
            send_metrics(start_time, first_token_time, tokens_generated, &tx);
            let _ = tx.send(Ok(StreamResponse {
                content: String::new(),
                reasoning_content: None,
                done: true,
            }));
        }
        Err(e) => {
            let _ = tx.send(Err(e));
        }
    }
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
fn process_input_tokens(ctx: &LlamaContext, input_tokens: &[i32]) -> Result<()> {
    if is_debug() {
        eprintln!(
            "🔥 About to decode input batch with {} tokens",
            input_tokens.len()
        );
    }

    if input_tokens.len() > 64 {
        let chunk_size = 64;
        let mut pos_offset = 0i32;
        for (i, chunk) in input_tokens.chunks(chunk_size).enumerate() {
            if is_debug() {
                eprintln!(
                    "  📦 Processing chunk {} with {} tokens (pos_offset: {})",
                    i + 1,
                    chunk.len(),
                    pos_offset
                );
            }
            let is_last_chunk = (i + 1) * chunk_size >= input_tokens.len();
            let batch = batch_get_one_with_offset(chunk, pos_offset, is_last_chunk);
            decode_batch(ctx, batch)
                .map_err(|e| Error::Config(format!("Failed to decode chunk {}: {}", i + 1, e)))?;
            pos_offset += i32::try_from(chunk.len()).unwrap_or(i32::MAX);
        }
    } else {
        let batch = batch_get_one_with_logits(input_tokens, true);
        decode_batch(ctx, batch)
            .map_err(|e| Error::Config(format!("Failed to decode input: {e}")))?;
    }

    if is_debug() {
        eprintln!("✅ Input batch decoded successfully");
    }

    Ok(())
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
fn validate_logits(ctx: &LlamaContext) -> Result<()> {
    let logits_ptr = ctx.get_logits_ith(-1);
    if logits_ptr.is_null() {
        return Err(Error::Config(
            "No logits available for sampling - decode step missing logits=1".to_string(),
        ));
    }
    Ok(())
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
fn send_metrics(
    start_time: Instant,
    first_token_time: Option<Instant>,
    tokens_generated: u32,
    _tx: &UnboundedSender<Result<StreamResponse>>,
) {
    let total_time = start_time.elapsed();
    let ttft = first_token_time.map(|t| t.duration_since(start_time));
    let tokens_per_sec = if total_time.as_secs_f64() > 0.0 {
        tokens_generated as f64 / total_time.as_secs_f64()
    } else {
        0.0
    };

    if is_debug() {
        let metrics_msg = if let Some(ttft) = ttft {
            format!(
                "\nGenerated {} tokens in {:.2}s ({:.1} tok/s, TTFT: {}ms)",
                tokens_generated,
                total_time.as_secs_f64(),
                tokens_per_sec,
                ttft.as_millis()
            )
        } else {
            format!(
                "\nGenerated {} tokens in {:.2}s ({:.1} tok/s)",
                tokens_generated,
                total_time.as_secs_f64(),
                tokens_per_sec
            )
        };

        tracing::info!("{}", metrics_msg);
        eprintln!("{metrics_msg}");
    }
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub(crate) async fn generate_tokens_with_vision(
    model: Arc<LlamaModel>,
    mtmd_ctx: Arc<MtmdContext>,
    messages: Vec<Message>,
    config: StreamingConfig,
    tx: UnboundedSender<Result<StreamResponse>>,
) {
    let result: Result<()> = async {
        let first_msg_with_image = messages
            .iter()
            .find(|m| m.images.is_some() && !m.images.as_ref().unwrap().is_empty())
            .ok_or_else(|| Error::Config("No images found in messages".to_string()))?;

        let image_b64 = &first_msg_with_image.images.as_ref().unwrap()[0];
        let image_bytes = decode_image(image_b64)?;

        if is_debug() {
            eprintln!("🖼️ Processing image: {} bytes", image_bytes.len());
        }

        let rgb_data = preprocess_image_for_clip(&image_bytes, 448, 448)
            .map_err(|e| Error::Config(format!("Image preprocessing failed: {e}")))?;

        let bitmap = MtmdBitmap::from_rgb(448, 448, &rgb_data)
            .map_err(|e| Error::Config(format!("Bitmap creation failed: {e}")))?;

        let marker = default_media_marker();
        let prompt_with_marker = format!("{} {}", marker, first_msg_with_image.content);

        if is_debug() {
            eprintln!("📝 Prompt with marker: {prompt_with_marker}");
        }

        let chunks = tokenize_with_images(&mtmd_ctx, &prompt_with_marker, &[&bitmap])
            .map_err(|e| Error::Config(format!("Tokenization failed: {e}")))?;

        if is_debug() {
            eprintln!("✅ Tokenized into {} chunks", chunks.size());
        }

        for i in 0..chunks.size() {
            if let Some(chunk) = chunks.get(i) {
                unsafe {
                    encode_chunk(&mtmd_ctx, chunk)
                        .map_err(|e| Error::Config(format!("Chunk encoding failed: {e}")))?;
                }

                let _embeddings_ptr = get_output_embeddings(&mtmd_ctx);
                if is_debug() {
                    eprintln!("🎯 Got embeddings for chunk {i}");
                }
            }
        }

        if is_debug() {
            eprintln!("🚀 Starting text generation after vision processing");
        }

        let dummy_prompt = format!("{}\n", first_msg_with_image.content);
        generate_tokens(model, dummy_prompt.into_bytes(), config, tx.clone()).await;

        Ok(())
    }
    .await;

    if let Err(e) = result {
        let _ = tx.send(Err(e));
    }
}
