use crate::{Error, Message, Result, StreamResponse, decode_image};
use arkavo_llama_cpp::{
    LlamaContext, LlamaModel, LlamaSampler, batch_free, batch_get_one_with_logits,
    batch_get_one_with_offset, batch_init_with_tokens, create_sampler_chain, decode_batch,
    token_to_piece, tokenize_with_model,
};
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llama_cpp::multimodal::{
    MtmdContext, MtmdBitmap, tokenize_with_images, encode_chunk,
    get_output_embeddings, preprocess_image_for_clip, default_media_marker,
};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;

static DEBUG_LLAMACPP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_debug(enabled: bool) {
    DEBUG_LLAMACPP.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

pub fn is_debug() -> bool {
    DEBUG_LLAMACPP.load(std::sync::atomic::Ordering::Relaxed)
}

#[derive(Debug, Clone)]
pub struct StreamingConfig {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: i32,
    pub max_tokens: u32,
    pub seed: u32,
}

pub async fn generate_tokens(
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

        let sampler = create_sampler_chain(
            config.temperature,
            config.top_p,
            config.top_k,
            config.seed,
        )
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

        for i in 0..max_generation {
            validate_logits(&ctx)?;

            let token = sampler.sample(&ctx, -1);
            if is_debug() {
                eprintln!("Sampled token: {token} at pos {pos}");
            }

            if token == eos_token {
                tracing::info!("Generation stopped at EOS token");
                break;
            }

            let piece = token_to_piece(vocab, token, false)
                .map_err(|e| Error::Config(format!("Failed to decode token: {e}")))?;

            if first_token_time.is_none() {
                first_token_time = Some(Instant::now());
            }

            tokens_generated += 1;

            let response = StreamResponse {
                content: piece,
                done: false,
            };

            if tx.send(Ok(response)).is_err() {
                break;
            }

            sampler.accept(token);

            let mut batch = batch_init_with_tokens(&[token], pos, true);
            decode_batch(&ctx, batch).map_err(|e| {
                Error::Config(format!("Failed to decode token at pos {pos}: {e}"))
            })?;
            batch_free(&mut batch);

            pos += 1;

            let total_tokens = input_tokens.len() + tokens_generated as usize;
            if total_tokens >= 32000 {
                tracing::info!(
                    "Generation stopped at context limit: {} tokens",
                    total_tokens
                );
                let _ = tx.send(Ok(StreamResponse {
                    content: String::new(),
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
                done: true,
            }));
        }
        Err(e) => {
            let _ = tx.send(Err(e));
        }
    }
}

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
            decode_batch(ctx, batch).map_err(|e| {
                Error::Config(format!("Failed to decode chunk {}: {}", i + 1, e))
            })?;
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

fn validate_logits(ctx: &LlamaContext) -> Result<()> {
    let logits_ptr = ctx.get_logits_ith(-1);
    if logits_ptr.is_null() {
        return Err(Error::Config(
            "No logits available for sampling - decode step missing logits=1".to_string(),
        ));
    }
    Ok(())
}

fn send_metrics(
    start_time: Instant,
    first_token_time: Option<Instant>,
    tokens_generated: u32,
    tx: &UnboundedSender<Result<StreamResponse>>,
) {
    let total_time = start_time.elapsed();
    let ttft = first_token_time.map(|t| t.duration_since(start_time));
    let tokens_per_sec = if total_time.as_secs_f64() > 0.0 {
        tokens_generated as f64 / total_time.as_secs_f64()
    } else {
        0.0
    };

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

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub async fn generate_tokens_with_vision(
    model: Arc<LlamaModel>,
    mtmd_ctx: Arc<MtmdContext>,
    messages: Vec<Message>,
    config: StreamingConfig,
    tx: UnboundedSender<Result<StreamResponse>>,
) {
    let result: Result<()> = async {
        let first_msg_with_image = messages.iter().find(|m| {
            m.images.is_some() && !m.images.as_ref().unwrap().is_empty()
        }).ok_or_else(|| Error::Config("No images found in messages".to_string()))?;

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
            eprintln!("📝 Prompt with marker: {}", prompt_with_marker);
        }

        let chunks = tokenize_with_images(&mtmd_ctx, &prompt_with_marker, &[&bitmap])
            .map_err(|e| Error::Config(format!("Tokenization failed: {e}")))?;

        if is_debug() {
            eprintln!("✅ Tokenized into {} chunks", chunks.size());
        }

        for i in 0..chunks.size() {
            if let Some(chunk) = chunks.get(i) {
                encode_chunk(&mtmd_ctx, chunk)
                    .map_err(|e| Error::Config(format!("Chunk encoding failed: {e}")))?;

                let embeddings_ptr = get_output_embeddings(&mtmd_ctx);
                if is_debug() {
                    eprintln!("🎯 Got embeddings for chunk {}", i);
                }
            }
        }

        if is_debug() {
            eprintln!("🚀 Starting text generation after vision processing");
        }

        let dummy_prompt = format!("{}\n", first_msg_with_image.content);
        generate_tokens(model, dummy_prompt.into_bytes(), config, tx.clone()).await;

        Ok(())
    }.await;

    if let Err(e) = result {
        let _ = tx.send(Err(e));
    }
}
