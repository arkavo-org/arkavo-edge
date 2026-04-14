#![allow(clippy::redundant_pub_crate)]

use crate::gpu_fault::classify_gpu_fault;
use crate::provider::InferenceTiming;
use crate::{Error, Message, Result, StreamResponse, decode_image};
use arkavo_llama_cpp::ModelFormat;
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llama_cpp::multimodal::{
    MtmdBitmap, MtmdContext, default_media_marker, encode_chunk, get_output_embeddings,
    preprocess_image_for_clip, tokenize_with_images,
};
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llama_cpp::{
    DrySamplingConfig, LlamaContext, LlamaModel, batch_free, batch_get_one_with_logits,
    batch_get_one_with_offset, batch_init_with_tokens, batch_init_with_tokens_seq,
    create_sampler_chain, create_sampler_chain_with_dry, decode_batch, perf_context,
    token_to_bytes, tokenize_with_model,
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

/// Return stop sequences that indicate the model is self-prompting (generating a new user turn).
/// These are chat template turn markers — a valid assistant response should never contain them.
fn stop_sequences_for_format(format: ModelFormat) -> &'static [&'static str] {
    match format {
        ModelFormat::Qwen3 => &["<|im_start|>user"],
        ModelFormat::Gemma3 | ModelFormat::Gemma4 => &["<start_of_turn>user"],
        ModelFormat::MistralV3 => &["[INST]"],
        ModelFormat::GLM4 => &["<|user|>"],
    }
}

/// Check if accumulated text contains a stop sequence indicating self-prompting.
/// Returns the byte offset of the stop sequence if found, or None.
fn detect_self_prompting(accumulated: &str, format: ModelFormat) -> Option<usize> {
    let stop_seqs = stop_sequences_for_format(format);
    for seq in stop_seqs {
        if let Some(pos) = accumulated.find(seq) {
            return Some(pos);
        }
    }
    None
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[derive(Debug, Clone)]
pub(crate) struct StreamingConfig {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: i32,
    pub max_tokens: u32,
    pub seed: u32,
    /// Enable dry sampling for repetition prevention (critical for GLM-4.7-Flash)
    pub use_dry_sampling: bool,
    /// Model format for stop-sequence detection (prevents self-prompting)
    pub model_format: ModelFormat,
    /// Optional GBNF grammar for constrained decoding (tool calling)
    pub grammar: Option<String>,
    /// Whether the grammar should activate lazily (on trigger) vs from the start
    pub grammar_lazy: bool,
    /// Typed triggers for lazy grammar activation (Token, Word, Pattern)
    pub grammar_triggers: Option<Vec<arkavo_llama_cpp::GrammarTrigger>>,
    /// Additional stop sequences from template engine (e.g., to prevent <think> blocks)
    pub additional_stops: Vec<String>,
    /// Generation prompt for non-lazy grammar prefilling (from Jinja template)
    pub generation_prompt: Option<String>,
}

/// Options for context reuse in multi-turn conversations
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[derive(Debug, Clone, Default)]
pub(crate) struct ContextReuseOptions {
    /// Starting position in KV cache (for resuming generation)
    /// If Some, skips processing tokens before this position
    pub start_position: Option<i32>,
    /// Whether to clear KV cache before generation
    pub clear_cache: bool,
    /// Sequence ID for multi-sequence inference. None = use default (seq 0).
    pub seq_id: Option<i32>,
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub(crate) async fn generate_tokens(
    model: Arc<LlamaModel>,
    prompt_bytes: Vec<u8>,
    config: StreamingConfig,
    tx: UnboundedSender<Result<StreamResponse>>,
) {
    generate_tokens_with_context(
        model,
        prompt_bytes,
        config,
        tx,
        ContextReuseOptions::default(),
    )
    .await;
}

/// Generate tokens using a pre-allocated pooled context.
/// Avoids context allocation contention by reusing an existing KV cache.
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub(crate) async fn generate_tokens_pooled(
    pooled_ctx: std::sync::Arc<std::sync::Mutex<LlamaContext>>,
    model: Arc<LlamaModel>,
    prompt_bytes: Vec<u8>,
    config: StreamingConfig,
    tx: UnboundedSender<Result<StreamResponse>>,
) {
    let start_time = Instant::now();
    let mut first_token_time: Option<Instant> = None;
    let mut tokens_generated = 0u32;

    let result = async {
        let ctx = pooled_ctx
            .lock()
            .map_err(|_| Error::Config("Context mutex poisoned".to_string()))?;
        // Reset KV cache and position tracking for fresh inference.
        // clear_kv_cache() is a no-op; use the memory API to actually reset.
        ctx.get_memory().clear(true);

        let vocab = model.get_vocab();
        let input_tokens = tokenize_with_model(vocab, &prompt_bytes)
            .map_err(|e| Error::Config(format!("Failed to tokenize: {e}")))?;

        tracing::info!("Input tokenized to {} tokens (pooled)", input_tokens.len());

        let sampler = if config.use_dry_sampling {
            let dry_config = DrySamplingConfig::for_glm();
            let vocab = model.get_vocab();
            #[allow(clippy::cast_possible_wrap)]
            let n_ctx_train = model.get_trained_context_size() as i32;
            unsafe {
                create_sampler_chain_with_dry(
                    config.temperature,
                    config.top_p,
                    config.top_k,
                    config.seed,
                    dry_config,
                    vocab,
                    n_ctx_train,
                )
            }
            .map_err(|e| Error::Config(format!("Failed to create sampler with dry: {e}")))?
        } else {
            create_sampler_chain(config.temperature, config.top_p, config.top_k, config.seed)
                .map_err(|e| Error::Config(format!("Failed to create sampler: {e}")))?
        };

        if let Some(ref grammar_str) = config.grammar {
            let vocab = model.get_vocab();
            if config.grammar_lazy {
                if let Some(ref triggers) = config.grammar_triggers {
                    unsafe {
                        sampler.add_grammar_lazy_with_triggers(
                            vocab,
                            grammar_str,
                            "root",
                            triggers,
                        );
                    }
                } else {
                    unsafe {
                        sampler.add_grammar(vocab, grammar_str, "root");
                    }
                }
            } else {
                unsafe {
                    sampler.add_grammar(vocab, grammar_str, "root");
                }
                // Prefill non-lazy grammar with generation_prompt tokens
                // so the grammar advances past the template-injected prefix.
                // This matches llama-server's sampling.cpp: common_tokenize(vocab, gen_prompt, false, true)
                if let Some(ref gen_prompt) = config.generation_prompt {
                    let vocab = model.get_vocab();
                    let mut toks = vec![0i32; gen_prompt.len() + 8];
                    let n = unsafe {
                        arkavo_llama_cpp::ffi::llama_tokenize(
                            vocab,
                            gen_prompt.as_ptr().cast::<std::os::raw::c_char>(),
                            i32::try_from(gen_prompt.len()).unwrap_or(i32::MAX),
                            toks.as_mut_ptr(),
                            i32::try_from(toks.len()).unwrap_or(i32::MAX),
                            false, // no BOS for grammar prefill
                            true,  // parse special tokens
                        )
                    };
                    if n > 0 {
                        let prefill_tokens = &toks[..n as usize];
                        // Strip leading space token if generation_prompt doesn't start with space
                        let start = if !prefill_tokens.is_empty() && !gen_prompt.starts_with(' ') {
                            let first_piece = arkavo_llama_cpp::detokenize(
                                vocab,
                                &[prefill_tokens[0]],
                                false,
                                true,
                            );
                            i32::from(first_piece.as_ref().is_some_and(|s| s.starts_with(' ')))
                        } else {
                            0
                        };
                        for token in &prefill_tokens[start..] {
                            sampler.accept(*token);
                        }
                    }
                }
            }
        }

        let eos_token = model.get_eos_token();
        process_input_tokens(&ctx, &input_tokens)?;
        let mut pos = i32::try_from(input_tokens.len()).unwrap_or(0);
        // Clamp generation to KV cache capacity: the allocated n_ctx (safe_ctx)
        // may be much smaller than max_tokens. Without this, generation crashes
        // with DecodeFailure when pos exceeds the KV cache boundary.
        let trained_ctx = model.get_trained_context_size();
        let safe_ctx = if trained_ctx <= 8192 {
            trained_ctx
        } else if trained_ctx <= 32768 {
            trained_ctx / 2
        } else {
            (trained_ctx / 4).min(16384)
        };
        let available = safe_ctx.saturating_sub(input_tokens.len() as u32);
        let max_generation = config.max_tokens.min(30000).min(available);
        let mut utf8_buffer: Vec<u8> = Vec::new();
        let mut detection_buffer = String::new();

        for _ in 0..max_generation {
            validate_logits(&ctx)?;
            let token = sampler.sample(&ctx, -1);

            if token == eos_token {
                tracing::info!("Generation stopped at EOS token");
                if !utf8_buffer.is_empty() {
                    let piece = String::from_utf8_lossy(&utf8_buffer).to_string();
                    let _ = tx.send(Ok(StreamResponse {
                        content: piece,
                        reasoning_content: None,
                        done: false,
                        inference_timing: None,
                    }));
                }
                break;
            }

            let special_bytes = token_to_bytes(vocab, token, true)
                .map_err(|e| Error::Config(format!("Failed to decode token (special): {e}")))?;
            if let Ok(s) = std::str::from_utf8(&special_bytes) {
                detection_buffer.push_str(s);
            }

            if !config.additional_stops.is_empty()
                && config
                    .additional_stops
                    .iter()
                    .any(|stop| detection_buffer.contains(stop))
            {
                tracing::info!("Generation stopped at stop sequence");
                if !utf8_buffer.is_empty() {
                    let piece = String::from_utf8_lossy(&utf8_buffer).to_string();
                    let _ = tx.send(Ok(StreamResponse {
                        content: piece,
                        reasoning_content: None,
                        done: false,
                        inference_timing: None,
                    }));
                }
                break;
            }

            let token_bytes = token_to_bytes(vocab, token, false)
                .map_err(|e| Error::Config(format!("Failed to decode token: {e}")))?;
            utf8_buffer.extend_from_slice(&token_bytes);
            let piece = extract_valid_utf8(&mut utf8_buffer);

            if first_token_time.is_none() && !piece.is_empty() {
                first_token_time = Some(Instant::now());
            }
            tokens_generated += 1;

            if !piece.is_empty()
                && tx
                    .send(Ok(StreamResponse {
                        content: piece,
                        reasoning_content: None,
                        done: false,
                        inference_timing: None,
                    }))
                    .is_err()
            {
                break;
            }

            sampler.accept(token);
            let mut batch = batch_init_with_tokens(&[token], pos, true);
            decode_batch(&ctx, batch)
                .map_err(|e| classify_decode_error(&format!("token at pos {pos}"), &e))?;
            batch_free(&mut batch);
            pos += 1;

            let total_tokens = input_tokens.len() + tokens_generated as usize;
            if total_tokens >= 32000 {
                tracing::info!(
                    "Generation stopped at context limit: {} tokens",
                    total_tokens
                );
                if !utf8_buffer.is_empty() {
                    let piece = String::from_utf8_lossy(&utf8_buffer).to_string();
                    let _ = tx.send(Ok(StreamResponse {
                        content: piece,
                        reasoning_content: None,
                        done: false,
                        inference_timing: None,
                    }));
                }
                break;
            }
        }

        let perf = perf_context(&ctx);
        drop(ctx);
        let timing = InferenceTiming {
            prompt_eval_ms: perf.t_p_eval_ms,
            generation_ms: perf.t_eval_ms,
            n_prompt_eval: perf.n_p_eval.max(0) as u32,
            n_eval: perf.n_eval.max(0) as u32,
        };
        Ok::<(u32, Option<Instant>, InferenceTiming), Error>((
            tokens_generated,
            first_token_time,
            timing,
        ))
    }
    .await;

    match result {
        Ok((tokens_generated, first_token_time, timing)) => {
            send_metrics(start_time, first_token_time, tokens_generated, &tx);
            let _ = tx.send(Ok(StreamResponse {
                content: String::new(),
                reasoning_content: None,
                done: true,
                inference_timing: Some(timing),
            }));
        }
        Err(e) => {
            let _ = tx.send(Err(e));
        }
    }
}

/// Generate tokens with optional context reuse for multi-turn conversations
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub(crate) async fn generate_tokens_with_context(
    model: Arc<LlamaModel>,
    prompt_bytes: Vec<u8>,
    config: StreamingConfig,
    tx: UnboundedSender<Result<StreamResponse>>,
    context_options: ContextReuseOptions,
) {
    let start_time = Instant::now();
    let mut first_token_time: Option<Instant> = None;
    let mut tokens_generated = 0u32;

    #[allow(clippy::significant_drop_tightening)]
    let result = async {
        let ctx = LlamaContext::new(&model)
            .map_err(|e| Error::Config(format!("Failed to create context: {e}")))?;

        // Clear KV cache if requested (for new conversations)
        if context_options.clear_cache {
            ctx.clear_kv_cache();
        }

        let vocab = model.get_vocab();
        let input_tokens = tokenize_with_model(vocab, &prompt_bytes)
            .map_err(|e| Error::Config(format!("Failed to tokenize: {e}")))?;

        tracing::info!("Input tokenized to {} tokens", input_tokens.len());
        if is_debug() {
            let tail_start = prompt_bytes.len().saturating_sub(300);
            if let Ok(tail) = std::str::from_utf8(&prompt_bytes[tail_start..]) {
                eprintln!("Prompt tail ({} bytes):\n«{tail}»", prompt_bytes.len());
            }
            eprintln!("Stops: {:?}, Grammar: {}", config.additional_stops, config.grammar.is_some());
        }
        if is_debug() {
            eprintln!(
                "🔍 First 10 tokens: {:?}",
                input_tokens.iter().take(10).collect::<Vec<_>>()
            );
        }

        // Use dry sampling to prevent repetition loops
        let sampler = if config.use_dry_sampling {
            let dry_config = DrySamplingConfig::for_glm(); // multiplier 1.1 works well across models
            let vocab = model.get_vocab();
            #[allow(clippy::cast_possible_wrap)]
            let n_ctx_train = model.get_trained_context_size() as i32;
            unsafe {
                create_sampler_chain_with_dry(
                    config.temperature,
                    config.top_p,
                    config.top_k,
                    config.seed,
                    dry_config,
                    vocab,
                    n_ctx_train,
                )
            }
            .map_err(|e| Error::Config(format!("Failed to create sampler with dry: {e}")))?
        } else {
            create_sampler_chain(config.temperature, config.top_p, config.top_k, config.seed)
                .map_err(|e| Error::Config(format!("Failed to create sampler: {e}")))?
        };

        // Add grammar sampler for constrained decoding of tool calls
        if let Some(ref grammar_str) = config.grammar {
            let vocab = model.get_vocab();
            if config.grammar_lazy {
                if let Some(ref triggers) = config.grammar_triggers {
                    unsafe {
                        sampler.add_grammar_lazy_with_triggers(
                            vocab,
                            grammar_str,
                            "root",
                            triggers,
                        );
                    }
                    tracing::debug!("Grammar sampler added (lazy, triggers: {triggers:?})");
                } else {
                    unsafe {
                        sampler.add_grammar(vocab, grammar_str, "root");
                    }
                    tracing::debug!("Grammar sampler added (strict, no triggers)");
                }
            } else {
                unsafe {
                    sampler.add_grammar(vocab, grammar_str, "root");
                }
                tracing::debug!("Grammar sampler added (strict)");
            }
        }

        let eos_token = model.get_eos_token();
        if is_debug() {
            eprintln!("EOS token from model: {eos_token}");
        }

        // If we have a start position, we're resuming - skip input processing
        let initial_pos = if let Some(start_pos) = context_options.start_position {
            start_pos
        } else {
            process_input_tokens(&ctx, &input_tokens)?;
            i32::try_from(input_tokens.len()).unwrap_or(0)
        };

        // Clamp generation to KV cache capacity (same logic as pooled path)
        let trained_ctx = model.get_trained_context_size();
        let safe_ctx = if trained_ctx <= 8192 {
            trained_ctx
        } else if trained_ctx <= 32768 {
            trained_ctx / 2
        } else {
            (trained_ctx / 4).min(16384)
        };
        let occupied = initial_pos as u32;
        let available = safe_ctx.saturating_sub(occupied);
        let max_generation = config.max_tokens.min(30000).min(available);
        let mut pos = initial_pos;

        if is_debug() {
            eprintln!(
                "🎯 Max generation tokens: {} (config.max_tokens: {})",
                max_generation, config.max_tokens
            );
        }

        // Buffer for incomplete UTF-8 sequences
        let mut utf8_buffer: Vec<u8> = Vec::new();
        // Detection buffer: decodes with special=true so template markers are visible
        let mut detection_buffer = String::new();

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
                        inference_timing: None,
                    }));
                }
                break;
            }

            // Decode with special=true for stop-sequence detection
            // Special tokens like <|im_start|> are only visible in this representation
            let mut special_text = String::new();
            if let Ok(special_bytes) = token_to_bytes(vocab, token, true)
                && let Ok(s) = std::str::from_utf8(&special_bytes)
            {
                special_text = s.to_string();
                detection_buffer.push_str(s);
            }

            // Inject think markers that are special tokens into the user-facing stream
            // so downstream consumers (process_stream) can detect and suppress them
            if special_text == "<think>" || special_text == "</think>" {
                let _ = tx.send(Ok(StreamResponse {
                    content: special_text,
                    reasoning_content: None,
                    done: false,
                    inference_timing: None,
                }));
            }

            // Check for additional stop sequences (e.g., <think> when thinking is disabled)
            let additional_stop = config.additional_stops.iter().find_map(|stop| {
                detection_buffer.find(stop.as_str()).map(|pos| (pos, stop.clone()))
            });
            if let Some((stop_pos, stop_seq)) = additional_stop {
                if is_debug() {
                    eprintln!(
                        "⛔ Additional stop detected: '{stop_seq}' at byte {stop_pos}"
                    );
                }
                tracing::info!("Generation stopped: additional stop sequence '{stop_seq}'");
                if !utf8_buffer.is_empty() {
                    let piece = extract_valid_utf8(&mut utf8_buffer);
                    if !piece.is_empty() {
                        let _ = tx.send(Ok(StreamResponse {
                            content: piece,
                            reasoning_content: None,
                            done: false,
                            inference_timing: None,
                        }));
                    }
                }
                break;
            }

            // Check for self-prompting using the detection buffer (with special tokens)
            if let Some(stop_pos) = detect_self_prompting(&detection_buffer, config.model_format) {
                if is_debug() {
                    eprintln!(
                        "⛔ Stop-sequence detected: model self-prompting at byte {stop_pos} in detection buffer"
                    );
                }
                tracing::info!("Generation stopped: self-prompting detected");
                // Flush any valid content remaining in utf8_buffer before the marker
                if !utf8_buffer.is_empty() {
                    let piece = extract_valid_utf8(&mut utf8_buffer);
                    if !piece.is_empty() {
                        let _ = tx.send(Ok(StreamResponse {
                            content: piece,
                            reasoning_content: None,
                            done: false,
                            inference_timing: None,
                        }));
                    }
                }
                break;
            }

            // Get raw bytes for this token (special=false for user-visible output)
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
                    inference_timing: None,
                };

                if tx.send(Ok(response)).is_err() {
                    break;
                }
            }

            sampler.accept(token);

            let mut batch = if let Some(sid) = context_options.seq_id {
                batch_init_with_tokens_seq(&[token], pos, sid, true)
            } else {
                batch_init_with_tokens(&[token], pos, true)
            };
            decode_batch(&ctx, batch)
                .map_err(|e| classify_decode_error(&format!("token at pos {pos}"), &e))?;
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
                        inference_timing: None,
                    }));
                }
                break;
            }
        }

        let perf = perf_context(&ctx);
        let timing = InferenceTiming {
            prompt_eval_ms: perf.t_p_eval_ms,
            generation_ms: perf.t_eval_ms,
            n_prompt_eval: perf.n_p_eval.max(0) as u32,
            n_eval: perf.n_eval.max(0) as u32,
        };

        Ok::<(u32, Option<Instant>, InferenceTiming), Error>((tokens_generated, first_token_time, timing))
    }
    .await;

    match result {
        Ok((tokens_generated, first_token_time, timing)) => {
            send_metrics(start_time, first_token_time, tokens_generated, &tx);
            let _ = tx.send(Ok(StreamResponse {
                content: String::new(),
                reasoning_content: None,
                done: true,
                inference_timing: Some(timing),
            }));
        }
        Err(e) => {
            let _ = tx.send(Err(e));
        }
    }
}

/// Classify a decode_batch error as either a GPU fault or a generic config error.
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
fn classify_decode_error(context: &str, raw: &str) -> Error {
    if let Some(kind) = classify_gpu_fault(raw) {
        Error::GpuFault {
            kind: format!("{kind}"),
            message: format!("{context}: {raw}"),
        }
    } else {
        Error::Config(format!("{context}: {raw}"))
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
                .map_err(|e| classify_decode_error(&format!("chunk {}", i + 1), &e))?;
            pos_offset += i32::try_from(chunk.len()).unwrap_or(i32::MAX);
        }
    } else {
        let batch = batch_get_one_with_logits(input_tokens, true);
        decode_batch(ctx, batch).map_err(|e| classify_decode_error("input batch", &e))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stop_sequences_qwen3() {
        let seqs = stop_sequences_for_format(ModelFormat::Qwen3);
        assert!(seqs.contains(&"<|im_start|>user"));
    }

    #[test]
    fn test_stop_sequences_gemma3() {
        let seqs = stop_sequences_for_format(ModelFormat::Gemma3);
        assert!(seqs.contains(&"<start_of_turn>user"));
    }

    #[test]
    fn test_stop_sequences_mistral() {
        let seqs = stop_sequences_for_format(ModelFormat::MistralV3);
        assert!(seqs.contains(&"[INST]"));
    }

    #[test]
    fn test_stop_sequences_glm4() {
        let seqs = stop_sequences_for_format(ModelFormat::GLM4);
        assert!(seqs.contains(&"<|user|>"));
    }

    #[test]
    fn test_detect_self_prompting_qwen3() {
        let text = "Here is a hello world script:\n```python\nprint('hello')\n```\n<|im_start|>user\nNow write one that takes input";
        let pos = detect_self_prompting(text, ModelFormat::Qwen3);
        assert!(pos.is_some());
        let offset = pos.unwrap();
        assert!(text[..offset].contains("print('hello')"));
        assert!(!text[..offset].contains("<|im_start|>user"));
    }

    #[test]
    fn test_detect_self_prompting_gemma3() {
        let text = "Hello world!\n<start_of_turn>user\nAnother prompt";
        assert!(detect_self_prompting(text, ModelFormat::Gemma3).is_some());
    }

    #[test]
    fn test_detect_self_prompting_mistral() {
        let text = "Some response</s>[INST] Another question [/INST]";
        assert!(detect_self_prompting(text, ModelFormat::MistralV3).is_some());
    }

    #[test]
    fn test_detect_self_prompting_glm4() {
        let text = "Code output here\n<|user|>\nWrite more code";
        assert!(detect_self_prompting(text, ModelFormat::GLM4).is_some());
    }

    #[test]
    fn test_no_false_positive_on_clean_response() {
        let text = "Here is your Python hello world:\n```python\nprint('Hello, World!')\n```\nThis script prints Hello, World! to the console.";
        assert!(detect_self_prompting(text, ModelFormat::Qwen3).is_none());
        assert!(detect_self_prompting(text, ModelFormat::Gemma3).is_none());
        assert!(detect_self_prompting(text, ModelFormat::MistralV3).is_none());
        assert!(detect_self_prompting(text, ModelFormat::GLM4).is_none());
    }

    #[test]
    fn test_no_false_positive_on_empty() {
        assert!(detect_self_prompting("", ModelFormat::Qwen3).is_none());
    }

    #[test]
    fn test_stop_position_is_start_of_marker() {
        let text = "Good response<|im_start|>user\nBad self-prompt";
        let pos = detect_self_prompting(text, ModelFormat::Qwen3).unwrap();
        assert_eq!(pos, "Good response".len());
    }

    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    #[test]
    fn test_classify_decode_error_gpu_fault() {
        let err =
            super::classify_decode_error("token at pos 42", "llama_decode failed with code: -3");
        assert!(matches!(err, crate::Error::GpuFault { .. }));
        assert!(err.to_string().contains("MetalKill"));
    }

    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    #[test]
    fn test_classify_decode_error_generic() {
        let err = super::classify_decode_error("token at pos 42", "some other error");
        assert!(matches!(err, crate::Error::Config(_)));
    }
}
