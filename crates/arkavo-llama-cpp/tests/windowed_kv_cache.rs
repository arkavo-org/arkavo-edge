#![cfg(not(target_env = "musl"))]
//! Text contexts against a real GGUF, created without a full-size
//! sliding-window cache. Runs only when `ARKAVO_TEST_TEXT_MODEL` points at a
//! model, so an ordinary `cargo test` stays offline:
//!
//!   ARKAVO_TEST_TEXT_MODEL=/path/to/model.gguf \
//!     cargo test -p arkavo-llama-cpp --test windowed_kv_cache
//!
//! A model without sliding-window layers (`sliding_window() == 0`) exercises
//! the same parameters but cannot show the windowed cache recycling cells,
//! and a recurrent or hybrid model cannot rewind at all. Each test prints
//! what the model it was given could exercise.

use arkavo_llama_cpp::{
    batch_free, batch_init_with_tokens, configured_context_length, decode_batch,
    tokenize_with_model, LlamaContext, LlamaModel,
};

const PASSAGE: &str = "The capital of France is Paris. The capital of Italy is Rome. \
                       The capital of Japan is Tokyo. The capital of Egypt is Cairo. ";

const QUESTION: &str = "The capital of Spain is";

/// Tokens sent per decode call, well under the smallest default batch.
const CHUNK_TOKENS: usize = 256;

/// Prompt tokens beyond the sliding window. A windowed cache holds the
/// window plus one micro-batch (512 by default), so a prompt this much
/// longer than the window forces it to recycle cells before generation.
const TOKENS_PAST_WINDOW: usize = 768;

fn model() -> Option<LlamaModel> {
    let path = std::env::var("ARKAVO_TEST_TEXT_MODEL").ok()?;
    assert!(
        std::path::Path::new(&path).exists(),
        "ARKAVO_TEST_TEXT_MODEL points at a missing file: {path}"
    );
    let model = LlamaModel::from_file(&path).expect("load model");
    eprintln!(
        "model {}: trained context {}, sliding window {}",
        model.model_name(),
        model.get_trained_context_size(),
        model.sliding_window()
    );
    Some(model)
}

/// Decodes `tokens` starting at `pos`, a chunk at a time, and returns the
/// position after the last one.
fn decode(ctx: &LlamaContext, tokens: &[i32], pos: i32) -> i32 {
    let mut next = pos;
    for chunk in tokens.chunks(CHUNK_TOKENS) {
        let mut batch = batch_init_with_tokens(chunk, next, true);
        decode_batch(ctx, batch).expect("decode");
        batch_free(&mut batch);
        next += i32::try_from(chunk.len()).expect("chunk length");
    }
    next
}

fn last_logits(ctx: &LlamaContext, model: &LlamaModel) -> Vec<f32> {
    let n_vocab = usize::try_from(model.n_vocab()).expect("vocab size");
    let ptr = ctx.get_logits_ith(-1);
    assert!(!ptr.is_null(), "no logits after decode");
    // SAFETY: llama.cpp returns a row of n_vocab floats for the last
    // position that requested logits, valid until the next decode.
    unsafe { std::slice::from_raw_parts(ptr, n_vocab) }.to_vec()
}

fn argmax(logits: &[f32]) -> i32 {
    let (index, _) = logits
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .expect("non-empty logits");
    i32::try_from(index).expect("token id")
}

/// A prompt long enough to outgrow the model's windowed cache.
fn prompt_tokens(model: &LlamaModel) -> Vec<i32> {
    let target = model.sliding_window() as usize + TOKENS_PAST_WINDOW;
    // Tokens per repetition vary with the tokenizer, so the passage is
    // repeated until the prompt is long enough rather than a computed
    // number of times.
    let mut repeats = 16;
    loop {
        let text = format!("{}{QUESTION}", PASSAGE.repeat(repeats));
        let tokens = tokenize_with_model(model.get_vocab(), text.as_bytes()).expect("tokenize");
        if tokens.len() >= target {
            return tokens;
        }
        repeats *= 2;
    }
}

#[test]
fn context_has_the_configured_length_and_generates() {
    let Some(model) = model() else {
        eprintln!("skipping: set ARKAVO_TEST_TEXT_MODEL");
        return;
    };
    let ctx = LlamaContext::new(&model).expect("context");

    let requested = configured_context_length(model.get_trained_context_size());
    assert_eq!(ctx.context_length(), requested.next_multiple_of(256));

    let prompt = prompt_tokens(&model);
    assert!(prompt.len() < ctx.context_length() as usize);
    let mut pos = decode(&ctx, &prompt, 0);

    let mut generated = Vec::new();
    for _ in 0..8 {
        let token = argmax(&last_logits(&ctx, &model));
        assert!((0..model.n_vocab()).contains(&token));
        generated.push(token);
        if model.is_eog(token) {
            break;
        }
        pos = decode(&ctx, &[token], pos);
    }
    assert!(!generated.is_empty());
    assert_eq!(ctx.get_memory().seq_pos_max(0), pos - 1);
}

/// Speculative decoding rewinds the unaccepted tail of the batch it just
/// decoded. The logits after the rewind must match a run that never decoded
/// the discarded tokens.
#[test]
fn rewinding_the_tail_of_the_last_batch_matches_a_clean_run() {
    let Some(model) = model() else {
        eprintln!("skipping: set ARKAVO_TEST_TEXT_MODEL");
        return;
    };
    if model.uses_mrope() {
        eprintln!("skipping: multi-axis RoPE models reject a batch that restarts a position");
        return;
    }
    let ctx = LlamaContext::new(&model).expect("context");
    let prompt = prompt_tokens(&model);

    let start = decode(&ctx, &prompt, 0);
    let accepted = argmax(&last_logits(&ctx, &model));
    decode(&ctx, &[accepted], start);
    let follow_up = argmax(&last_logits(&ctx, &model));
    decode(&ctx, &[follow_up], start + 1);
    let clean = last_logits(&ctx, &model);

    ctx.get_memory().clear(true);
    decode(&ctx, &prompt, 0);
    // The accepted token followed by three drafts the sampler then rejects.
    let rejected = [prompt[0], prompt[1], prompt[2]];
    let speculative = [accepted, rejected[0], rejected[1], rejected[2]];
    decode(&ctx, &speculative, start);
    if !ctx.get_memory().seq_rm(0, start + 1, -1) {
        // Recurrent and hybrid models keep state that cannot be rewound, with
        // or without a windowed cache; generation never rewinds those.
        eprintln!("skipping: this model's memory refuses partial removal");
        return;
    }
    assert_eq!(ctx.get_memory().seq_pos_max(0), start);
    decode(&ctx, &[follow_up], start + 1);
    let rewound = last_logits(&ctx, &model);

    assert_eq!(argmax(&rewound), argmax(&clean));
    let widest = clean
        .iter()
        .zip(&rewound)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    // Batches of different sizes take different GPU kernels, so the logits
    // agree to rounding rather than bit for bit.
    assert!(widest < 0.05, "logits diverged by {widest} after rewind");
}
