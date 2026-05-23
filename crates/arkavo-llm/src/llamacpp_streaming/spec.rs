//! NGRAM speculative-decoding generation paths.
//!
//! These functions are extracted from the parent module so the parent stays
//! under the 400-line implementation guideline. They are functionally
//! identical to the original code; only the file boundary moved.

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use crate::provider::InferenceTiming;
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use crate::{Error, Result, StreamResponse};
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llama_cpp::ModelFormat;
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llama_cpp::speculative::SpeculativeContext;
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llama_cpp::{
    DrySamplingConfig, LlamaContext, LlamaModel, batch_free, batch_init_with_tokens,
    batch_init_with_tokens_seq, create_sampler_chain, create_sampler_chain_with_dry, decode_batch,
    perf_context, token_to_bytes, tokenize_with_model,
};
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use std::sync::Arc;
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use std::time::Instant;
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use tokio::sync::mpsc::UnboundedSender;

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use super::{
    ContextReuseOptions, StreamingConfig, classify_decode_error, detect_self_prompting,
    extract_valid_utf8, process_input_tokens, send_metrics, validate_logits,
};

/// Outcome of emitting a single token through the shared stream/stop pipeline.
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub(super) struct EmitOutcome {
    /// True if a stop condition (self-prompting) was detected after this token.
    pub(super) stopped: bool,
}

/// Decode a sampled token to bytes, run self-prompting detection, and send
/// the user-visible piece through the stream channel. Shared by the spec
/// path's emission of both target and accepted-draft tokens so they go
/// through exactly the same pipeline the baseline path would.
///
/// Returns `Ok(EmitOutcome { stopped: true })` when self-prompting is
/// detected (caller should break the generation loop).
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub(super) fn emit_token(
    vocab: *const arkavo_llama_cpp::ffi::llama_vocab,
    token: i32,
    utf8_buffer: &mut Vec<u8>,
    detection_buffer: &mut String,
    model_format: ModelFormat,
    first_token_time: &mut Option<Instant>,
    tx: &UnboundedSender<Result<StreamResponse>>,
) -> Result<EmitOutcome> {
    // Special-token decoding for stop-sequence detection.
    let mut special_text = String::new();
    if let Ok(special_bytes) = token_to_bytes(vocab, token, true)
        && let Ok(s) = std::str::from_utf8(&special_bytes)
    {
        special_text = s.to_string();
        detection_buffer.push_str(s);
    }

    if special_text == "<think>" || special_text == "</think>" {
        let _ = tx.send(Ok(StreamResponse {
            content: special_text,
            reasoning_content: None,
            done: false,
            inference_timing: None,
        }));
    }

    if detect_self_prompting(detection_buffer, model_format).is_some() {
        if !utf8_buffer.is_empty() {
            let piece = extract_valid_utf8(utf8_buffer);
            if !piece.is_empty() {
                let _ = tx.send(Ok(StreamResponse {
                    content: piece,
                    reasoning_content: None,
                    done: false,
                    inference_timing: None,
                }));
            }
        }
        return Ok(EmitOutcome { stopped: true });
    }

    let token_bytes = token_to_bytes(vocab, token, false)
        .map_err(|e| Error::Config(format!("Failed to decode token: {e}")))?;
    utf8_buffer.extend_from_slice(&token_bytes);
    let piece = extract_valid_utf8(utf8_buffer);

    if first_token_time.is_none() && !piece.is_empty() {
        *first_token_time = Some(Instant::now());
    }

    if !piece.is_empty() {
        let _ = tx.send(Ok(StreamResponse {
            content: piece,
            reasoning_content: None,
            done: false,
            inference_timing: None,
        }));
    }

    Ok(EmitOutcome { stopped: false })
}

/// Spec-decoding variant of the pooled generation path.
///
/// At each step drafts up to `N_DRAFT_MAX` tokens via NGRAM_SIMPLE, decodes a
/// multi-token batch with logits at every position, and accepts the longest
/// prefix that matches the sampler's own choice. Sampling parity at
/// temperature 0.0 is preserved because the same sampler is invoked at each
/// position; spec only changes how many tokens are decoded per batch.
///
/// Only engaged when grammar is None and additional_stops is empty —
/// `generate_tokens_pooled` enforces that gate.
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub(super) async fn generate_tokens_pooled_with_spec(
    pooled_ctx: std::sync::Arc<std::sync::Mutex<LlamaContext>>,
    model: Arc<LlamaModel>,
    prompt_bytes: Vec<u8>,
    config: StreamingConfig,
    tx: UnboundedSender<Result<StreamResponse>>,
) {
    /// Upstream default n_max for ngram_simple.
    const N_DRAFT_MAX: i32 = 8;

    let start_time = Instant::now();
    let mut first_token_time: Option<Instant> = None;
    let mut tokens_generated = 0u32;
    let mut n_draft_total: u32 = 0;
    let mut n_accepted_total: u32 = 0;

    let result = async {
        let ctx = pooled_ctx
            .lock()
            .map_err(|_| Error::Config("Context mutex poisoned".to_string()))?;
        // Reset KV cache and position tracking for fresh inference.
        ctx.get_memory().clear(true);

        let vocab = model.get_vocab();
        let input_tokens = tokenize_with_model(vocab, &prompt_bytes)
            .map_err(|e| Error::Config(format!("Failed to tokenize: {e}")))?;

        tracing::info!(
            "Input tokenized to {} tokens (pooled spec path)",
            input_tokens.len()
        );

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

        let eos_token = model.get_eos_token();
        // Pooled path is always sequence 0; there is no caller-provided
        // seq_id option (unlike the non-pooled context-reuse variant).
        let seq_id: i32 = 0;

        process_input_tokens(&ctx, &input_tokens)?;
        let start_pos = i32::try_from(input_tokens.len()).unwrap_or(0);

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

        let mut spec_ctx = SpeculativeContext::new_ngram(1)
            .map_err(|e| Error::Config(format!("Failed to init speculative context: {e}")))?;
        spec_ctx.begin(seq_id, &input_tokens);

        // Rolling token history fed to the ngram drafter. Starts with the
        // prompt tokens; updated each iteration with accepted output tokens.
        let mut history: Vec<i32> = input_tokens;

        let mut utf8_buffer: Vec<u8> = Vec::new();
        let mut detection_buffer = String::new();

        let mut pos = start_pos;
        let end_pos = start_pos.saturating_add(i32::try_from(max_generation).unwrap_or(i32::MAX));

        // The first sample of each iteration normally comes from the model
        // (sampler.sample(&ctx, -1)). When the previous iteration's spec
        // verification ended with a divergent token we already sampled it
        // mid-batch, so we feed it forward via `pre_sampled` rather than
        // calling the sampler a second time.
        let mut pre_sampled: Option<i32> = None;

        while pos < end_pos {
            validate_logits(&ctx)?;

            let target_token = match pre_sampled.take() {
                Some(t) => t,
                None => sampler.sample(&ctx, -1),
            };

            if target_token == eos_token {
                tracing::info!("Generation stopped at EOS token (pooled spec)");
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

            // Emit target_token through the shared stream/stop pipeline.
            let emit = emit_token(
                vocab,
                target_token,
                &mut utf8_buffer,
                &mut detection_buffer,
                config.model_format,
                &mut first_token_time,
                &tx,
            )?;
            tokens_generated += 1;
            if emit.stopped {
                break;
            }
            sampler.accept(target_token);

            // Draft candidate continuation tokens from the running history.
            let drafts = spec_ctx.draft(seq_id, pos + 1, target_token, N_DRAFT_MAX, &history);

            if drafts.is_empty() {
                // No spec opportunity — fall back to the standard single-token
                // decode. Pooled path always operates on seq 0, so we use the
                // non-seq batch init (same as the baseline pooled path).
                let mut batch = batch_init_with_tokens(&[target_token], pos, true);
                decode_batch(&ctx, batch)
                    .map_err(|e| classify_decode_error(&format!("token at pos {pos}"), &e))?;
                batch_free(&mut batch);

                history.push(target_token);
                pos += 1;
            } else {
                n_draft_total += drafts.len() as u32;

                // Build a batch [target_token@pos, draft0@pos+1, ...] with
                // logits requested at every position so we can verify drafts.
                let mut spec_tokens: Vec<i32> = Vec::with_capacity(1 + drafts.len());
                spec_tokens.push(target_token);
                spec_tokens.extend_from_slice(&drafts);
                let mut batch =
                    arkavo_llama_cpp::batch_init_with_tokens_all_logits(&spec_tokens, pos, seq_id);
                decode_batch(&ctx, batch).map_err(|e| {
                    classify_decode_error(&format!("pooled spec batch at pos {pos}"), &e)
                })?;
                batch_free(&mut batch);

                // Walk drafts: at batch index i, the logits predict the token
                // that should follow spec_tokens[i]. So index 0 predicts the
                // token after target_token, which should match drafts[0] if
                // the speculation is correct.
                let mut n_accepted: usize = 0;
                let mut divergent: Option<i32> = None;
                // `i` is bounded by drafts.len() (max N_DRAFT_MAX=8), so the
                // cast is safe; clippy can't see the bound at type level.
                for (i, &drafted) in drafts.iter().enumerate() {
                    let logits_idx = i32::try_from(i).unwrap_or(i32::MAX);
                    let sampled_at_i = sampler.sample(&ctx, logits_idx);
                    if sampled_at_i == drafted {
                        sampler.accept(drafted);
                        n_accepted += 1;
                    } else {
                        divergent = Some(sampled_at_i);
                        break;
                    }
                }
                n_accepted_total += n_accepted as u32;
                spec_ctx.accept(seq_id, n_accepted as u16);

                // Append target_token to history; it's been emitted above.
                history.push(target_token);
                pos += 1;

                // Emit accepted draft tokens through the shared pipeline.
                let mut early_stop = false;
                for &accepted_tok in &drafts[..n_accepted] {
                    let e = emit_token(
                        vocab,
                        accepted_tok,
                        &mut utf8_buffer,
                        &mut detection_buffer,
                        config.model_format,
                        &mut first_token_time,
                        &tx,
                    )?;
                    tokens_generated += 1;
                    history.push(accepted_tok);
                    pos += 1;
                    if accepted_tok == eos_token || e.stopped {
                        early_stop = true;
                        break;
                    }
                }
                if early_stop {
                    break;
                }

                // Roll back KV positions occupied by unaccepted drafts so the
                // next iteration's sample(&ctx, -1) reads logits at the right
                // position. We removed `drafts.len() - n_accepted` positions
                // from the tail of the batch, starting at `pos` (the next
                // position to write).
                if n_accepted < drafts.len() {
                    ctx.get_memory().seq_rm(seq_id, pos, -1);
                    // We already sampled the divergent token at batch idx
                    // n_accepted — forward it to the next iteration so the
                    // sampler isn't run twice on the same logits.
                    pre_sampled = divergent;
                }
            }

            let total_tokens = history.len();
            if total_tokens >= 32000 {
                tracing::info!(
                    "Generation stopped at context limit: {} tokens (pooled spec)",
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
            n_thinking_eval: None,
            n_draft: Some(n_draft_total),
            n_accepted: Some(n_accepted_total),
            spec_bypassed: None,
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

/// Spec-decoding variant of `generate_tokens_baseline`.
///
/// Runs the normal sample/decode loop, but at each step:
///   1. drafts up to N tokens via the NGRAM_SIMPLE speculative context, and
///   2. if drafts are produced, decodes `[target_token, ...drafts]` in a
///      single batch, samples at each draft position, and accepts the
///      longest prefix that matches the sampler's own choice.
///
/// Sampling parity vs the baseline path is preserved at temperature 0.0
/// because the *exact same sampler* is invoked at each position; spec only
/// changes how many tokens are decoded per batch, never how a token is
/// chosen given a particular logits distribution.
///
/// Only engaged when grammar is None and additional_stops is empty —
/// `generate_tokens_with_context` enforces that gate.
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub(super) async fn generate_tokens_with_spec(
    model: Arc<LlamaModel>,
    prompt_bytes: Vec<u8>,
    config: StreamingConfig,
    tx: UnboundedSender<Result<StreamResponse>>,
    context_options: ContextReuseOptions,
) {
    /// Upstream default n_max for ngram_simple.
    const N_DRAFT_MAX: i32 = 8;

    let start_time = Instant::now();
    let mut first_token_time: Option<Instant> = None;
    let mut tokens_generated = 0u32;
    let mut n_draft_total: u32 = 0;
    let mut n_accepted_total: u32 = 0;

    #[allow(clippy::significant_drop_tightening)]
    let result = async {
        let ctx = LlamaContext::new(&model)
            .map_err(|e| Error::Config(format!("Failed to create context: {e}")))?;

        if context_options.clear_cache {
            ctx.clear_kv_cache();
        }

        let vocab = model.get_vocab();
        let input_tokens = tokenize_with_model(vocab, &prompt_bytes)
            .map_err(|e| Error::Config(format!("Failed to tokenize: {e}")))?;

        tracing::info!(
            "Input tokenized to {} tokens (spec path)",
            input_tokens.len()
        );

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

        let eos_token = model.get_eos_token();
        let seq_id = context_options.seq_id.unwrap_or(0);

        let initial_pos = if let Some(start_pos) = context_options.start_position {
            start_pos
        } else {
            process_input_tokens(&ctx, &input_tokens)?;
            i32::try_from(input_tokens.len()).unwrap_or(0)
        };

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

        // Build the spec ngram cache with prior-turn context when resuming,
        // or with just the prompt tokens for a fresh start.
        // When start_position is Some and prior_tokens is non-empty, prefix the
        // history with prior-turn output so the ngram drafter has cross-turn
        // patterns. Empty prior_tokens means the cache has no cross-turn
        // context — spec performance is degraded but correctness is preserved.
        let history_seed: Vec<i32> = if context_options.start_position.is_some()
            && !context_options.prior_tokens.is_empty()
        {
            let mut h = Vec::with_capacity(context_options.prior_tokens.len() + input_tokens.len());
            h.extend(context_options.prior_tokens.iter().copied());
            h.extend(input_tokens.iter().copied());
            h
        } else {
            input_tokens
        };

        let mut spec_ctx = SpeculativeContext::new_ngram(1)
            .map_err(|e| Error::Config(format!("Failed to init speculative context: {e}")))?;
        spec_ctx.begin(seq_id, &history_seed);

        // Rolling token history fed to the ngram drafter. Starts with the
        // seeded history (prior tokens + prompt tokens, or just prompt tokens);
        // updated each iteration with accepted output tokens.
        let mut history: Vec<i32> = history_seed;

        let mut utf8_buffer: Vec<u8> = Vec::new();
        let mut detection_buffer = String::new();
        let end_pos = initial_pos.saturating_add(i32::try_from(max_generation).unwrap_or(i32::MAX));

        // The first sample of each iteration normally comes from the model
        // (sampler.sample(&ctx, -1)). When the previous iteration's spec
        // verification ended with a divergent token we already sampled it
        // mid-batch, so we feed it forward via `pre_sampled` rather than
        // calling the sampler a second time.
        let mut pre_sampled: Option<i32> = None;

        while pos < end_pos {
            validate_logits(&ctx)?;

            let target_token = match pre_sampled.take() {
                Some(t) => t,
                None => sampler.sample(&ctx, -1),
            };

            if target_token == eos_token {
                tracing::info!("Generation stopped at EOS token (spec)");
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

            // Emit target_token through the shared stream/stop pipeline.
            let emit = emit_token(
                vocab,
                target_token,
                &mut utf8_buffer,
                &mut detection_buffer,
                config.model_format,
                &mut first_token_time,
                &tx,
            )?;
            tokens_generated += 1;
            if emit.stopped {
                break;
            }
            sampler.accept(target_token);

            // Draft candidate continuation tokens from the running history.
            let drafts = spec_ctx.draft(seq_id, pos + 1, target_token, N_DRAFT_MAX, &history);

            if drafts.is_empty() {
                // No spec opportunity this step — fall back to the standard
                // single-token decode.
                let mut batch = if let Some(sid) = context_options.seq_id {
                    batch_init_with_tokens_seq(&[target_token], pos, sid, true)
                } else {
                    batch_init_with_tokens(&[target_token], pos, true)
                };
                decode_batch(&ctx, batch)
                    .map_err(|e| classify_decode_error(&format!("token at pos {pos}"), &e))?;
                batch_free(&mut batch);

                history.push(target_token);
                pos += 1;
            } else {
                n_draft_total += drafts.len() as u32;

                // Build a batch [target_token@pos, draft0@pos+1, ...] with
                // logits requested at every position so we can verify drafts.
                let mut spec_tokens: Vec<i32> = Vec::with_capacity(1 + drafts.len());
                spec_tokens.push(target_token);
                spec_tokens.extend_from_slice(&drafts);
                let mut batch =
                    arkavo_llama_cpp::batch_init_with_tokens_all_logits(&spec_tokens, pos, seq_id);
                decode_batch(&ctx, batch)
                    .map_err(|e| classify_decode_error(&format!("spec batch at pos {pos}"), &e))?;
                batch_free(&mut batch);

                // Walk drafts: at batch index i, the logits predict the token
                // that should follow spec_tokens[i]. So index 0 predicts the
                // token after target_token, which should match drafts[0] if
                // the speculation is correct.
                let mut n_accepted: usize = 0;
                let mut divergent: Option<i32> = None;
                // `i` is bounded by drafts.len() (max N_DRAFT_MAX=8), so the
                // cast is safe; clippy can't see the bound at type level.
                for (i, &drafted) in drafts.iter().enumerate() {
                    let logits_idx = i32::try_from(i).unwrap_or(i32::MAX);
                    let sampled_at_i = sampler.sample(&ctx, logits_idx);
                    if sampled_at_i == drafted {
                        sampler.accept(drafted);
                        n_accepted += 1;
                    } else {
                        divergent = Some(sampled_at_i);
                        break;
                    }
                }
                n_accepted_total += n_accepted as u32;
                spec_ctx.accept(seq_id, n_accepted as u16);

                // Append target_token to history; it's been emitted above.
                history.push(target_token);
                pos += 1;

                // Emit accepted draft tokens through the shared pipeline.
                let mut early_stop = false;
                for &accepted_tok in &drafts[..n_accepted] {
                    let e = emit_token(
                        vocab,
                        accepted_tok,
                        &mut utf8_buffer,
                        &mut detection_buffer,
                        config.model_format,
                        &mut first_token_time,
                        &tx,
                    )?;
                    tokens_generated += 1;
                    history.push(accepted_tok);
                    pos += 1;
                    if accepted_tok == eos_token || e.stopped {
                        early_stop = true;
                        break;
                    }
                }
                if early_stop {
                    break;
                }

                // Roll back KV positions occupied by unaccepted drafts so the
                // next iteration's sample(&ctx, -1) reads logits at the right
                // position. We removed `drafts.len() - n_accepted` positions
                // from the tail of the batch, starting at `pos` (the next
                // position to write).
                if n_accepted < drafts.len() {
                    ctx.get_memory().seq_rm(seq_id, pos, -1);
                    // We already sampled the divergent token at batch idx
                    // n_accepted — forward it to the next iteration so the
                    // sampler isn't run twice on the same logits.
                    pre_sampled = divergent;
                }
            }

            let total_tokens = history.len();
            if total_tokens >= 32000 {
                tracing::info!(
                    "Generation stopped at context limit: {} tokens (spec)",
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
        let timing = InferenceTiming {
            prompt_eval_ms: perf.t_p_eval_ms,
            generation_ms: perf.t_eval_ms,
            n_prompt_eval: perf.n_p_eval.max(0) as u32,
            n_eval: perf.n_eval.max(0) as u32,
            n_thinking_eval: None,
            n_draft: Some(n_draft_total),
            n_accepted: Some(n_accepted_total),
            spec_bypassed: None,
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
