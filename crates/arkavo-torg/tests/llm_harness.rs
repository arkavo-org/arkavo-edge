//! Test harness for LLM-based TØR-G integration tests
//!
//! Provides model loading, prompt formatting, and constrained generation utilities.
//! Supports multiple model families: Qwen3 and Ministral.

#![cfg(not(target_env = "musl"))]
#![allow(dead_code)] // Harness functions may not all be used in every test
#![allow(unreachable_pub)] // Test module visibility doesn't matter
#![allow(clippy::missing_panics_doc)] // Test harness doesn't need panic docs
#![allow(clippy::cast_possible_wrap)] // Token counts won't exceed i32

use std::path::PathBuf;

use arkavo_llama_cpp::{
    LlamaContext, LlamaModel, LlamaSampler, ModelFormat, batch_free, batch_init_with_tokens,
    decode_batch, detect_model_format, tokenize_with_model,
};
use arkavo_torg::{
    MinistralTokenMap, Qwen3TokenMap, TorgError, TorgLlamaSampler, format_prompt,
};
use torg_core::Graph;
use torg_mask::TokenMapping;

const DEFAULT_MODEL_PATH: &str = ".cache/arkavo/models/qwen3-0.6b.gguf";

/// Get the model path from environment or default location
pub fn model_path() -> PathBuf {
    std::env::var("ARKAVO_TORG_MODEL_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .expect("Could not find home directory")
                .join(DEFAULT_MODEL_PATH)
        })
}

/// Load the model if available, returning None if not found
pub fn load_model_if_available() -> Option<LlamaModel> {
    let path = model_path();
    if !path.exists() {
        eprintln!(
            "Model not found at {}, skipping test. \
             Set ARKAVO_TORG_MODEL_PATH to override.",
            path.display()
        );
        return None;
    }
    match LlamaModel::from_file(path.to_str()?) {
        Ok(model) => Some(model),
        Err(e) => {
            eprintln!("Failed to load model: {e}");
            None
        }
    }
}

// Note: TORG_SYSTEM_PROMPT and format_prompt are now imported from arkavo_torg

/// Build a token mapping appropriate for the model format
///
/// # Safety
///
/// The `vocab` pointer must be valid and point to an initialized llama_vocab.
pub unsafe fn build_token_map(
    vocab: *const arkavo_llama_cpp::ffi::llama_vocab,
    format: ModelFormat,
) -> Result<(TokenMapping, i32), TorgError> {
    match format {
        ModelFormat::MistralV3 => {
            // SAFETY: caller guarantees vocab is valid
            let map = unsafe { MinistralTokenMap::from_vocab(vocab)? };
            let vocab_size = map.vocab_size();
            Ok((map.into_mapping(), vocab_size))
        }
        _ => {
            // Default to Qwen3 mapping for other formats
            // SAFETY: caller guarantees vocab is valid
            let map = unsafe { Qwen3TokenMap::from_vocab(vocab)? };
            let vocab_size = map.vocab_size();
            Ok((map.into_mapping(), vocab_size))
        }
    }
}

/// Error type for generation failures
#[derive(Debug)]
pub enum GenerationError {
    Tokenization(String),
    Decode(String),
    Sampler(String),
    Torg(TorgError),
    MaxTokensReached,
}

impl std::fmt::Display for GenerationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GenerationError::Tokenization(e) => write!(f, "Tokenization error: {e}"),
            GenerationError::Decode(e) => write!(f, "Decode error: {e}"),
            GenerationError::Sampler(e) => write!(f, "Sampler error: {e}"),
            GenerationError::Torg(e) => write!(f, "TØR-G error: {e}"),
            GenerationError::MaxTokensReached => write!(f, "Max tokens reached without completion"),
        }
    }
}

impl std::error::Error for GenerationError {}

impl From<TorgError> for GenerationError {
    fn from(e: TorgError) -> Self {
        GenerationError::Torg(e)
    }
}

/// Generate a TØR-G graph using constrained decoding
///
/// # Arguments
///
/// * `model` - The loaded LLM model
/// * `ctx` - The model context
/// * `policy` - Natural language policy description
/// * `max_tokens` - Maximum tokens to generate
///
/// # Returns
///
/// The generated TØR-G graph, or an error if generation fails.
pub fn generate_with_constraints(
    model: &LlamaModel,
    ctx: &LlamaContext,
    policy: &str,
    max_tokens: usize,
) -> Result<Graph, GenerationError> {
    let vocab = model.get_vocab();

    // Detect model format from path
    let model_path = model_path();
    let model_name = model_path.to_string_lossy();
    let format = detect_model_format(&model_name);
    eprintln!("Detected model format: {:?}", format);

    // Build token mapping appropriate for this model
    let (mapping, vocab_size) =
        unsafe { build_token_map(vocab, format).map_err(GenerationError::Torg)? };
    let mut torg_sampler = TorgLlamaSampler::new(mapping, vocab_size);

    // Format and tokenize prompt with correct chat template
    let prompt = format_prompt(policy, format);
    eprintln!("Formatted prompt:\n{}", &prompt[..prompt.len().min(200)]);
    let tokens =
        tokenize_with_model(vocab, prompt.as_bytes()).map_err(GenerationError::Tokenization)?;

    // Process prompt through model
    let mut batch = batch_init_with_tokens(&tokens, 0, true);
    decode_batch(ctx, batch).map_err(GenerationError::Decode)?;
    batch_free(&mut batch);

    let mut pos = tokens.len() as i32;
    let eos = model.get_eos_token();

    // Generation loop with constrained sampling
    for _ in 0..max_tokens {
        // Get TØR-G constraints FIRST
        let bias = torg_sampler.get_logit_bias();

        // Create sampler chain with CORRECT ORDER:
        // 1. Logit bias (constraint enforcement) - MUST BE FIRST
        // 2. Temperature
        // 3. Greedy selection
        let llama_sampler = LlamaSampler::new_chain(true).map_err(GenerationError::Sampler)?;

        // Add logit bias FIRST to mask disallowed tokens
        llama_sampler.add_logit_bias(torg_sampler.vocab_size(), &bias);

        // Then temperature (0.0 for greedy/deterministic)
        llama_sampler.add_temp(0.0);

        // Finally, greedy selection
        llama_sampler.add_greedy();

        // Sample next token
        let token = llama_sampler.sample(ctx, -1);

        // Debug: show allowed tokens (first iteration only)
        let allowed = torg_sampler.allowed_tokens();
        let is_allowed = allowed.contains(&(token as u32));
        if pos == tokens.len() as i32 {
            eprintln!("Initial allowed tokens ({}):", allowed.len());
            for &tid in allowed.iter().take(20) {
                let s =
                    arkavo_llama_cpp::token_to_piece(vocab, tid as i32, true).unwrap_or_default();
                eprintln!("  {} '{}'", tid, s.escape_debug());
            }
        }

        // CRITICAL: Verify mask is working - sampled token MUST be allowed
        assert!(
            is_allowed,
            "MASK FAILURE: Sampled token {} not in allowed set {:?}",
            token,
            &allowed[..allowed.len().min(10)]
        );

        // Check for completion
        if token == eos || torg_sampler.is_complete() {
            break;
        }

        // Feed token to TØR-G state machine
        let token_str = arkavo_llama_cpp::token_to_piece(vocab, token, true).unwrap_or_default();
        eprintln!(
            "Feeding token {} '{}' to TØR-G (allowed: {})",
            token, token_str, is_allowed
        );
        if let Err(e) = torg_sampler.feed_token(token as u32) {
            eprintln!("feed_token failed: {:?}", e);
            return Err(e.into());
        }

        // Advance model context
        let mut batch = batch_init_with_tokens(&[token], pos, true);
        decode_batch(ctx, batch).map_err(GenerationError::Decode)?;
        batch_free(&mut batch);
        pos += 1;
    }

    if !torg_sampler.is_complete() {
        return Err(GenerationError::MaxTokensReached);
    }

    Ok(torg_sampler.finish()?)
}

/// Verify OR gate behavior with truth table
pub fn verify_or_behavior(graph: &torg_core::Graph, output_id: u16) {
    use std::collections::HashMap;
    use torg_core::evaluate;

    // OR truth table: T,F→T; F,T→T; F,F→F; T,T→T
    let inputs_map =
        |a: bool, b: bool| -> HashMap<u16, bool> { [(0, a), (1, b)].into_iter().collect() };

    assert!(
        evaluate(graph, &inputs_map(true, false)).unwrap()[&output_id],
        "OR(T,F) should be true"
    );
    assert!(
        evaluate(graph, &inputs_map(false, true)).unwrap()[&output_id],
        "OR(F,T) should be true"
    );
    assert!(
        !evaluate(graph, &inputs_map(false, false)).unwrap()[&output_id],
        "OR(F,F) should be false"
    );
    assert!(
        evaluate(graph, &inputs_map(true, true)).unwrap()[&output_id],
        "OR(T,T) should be true"
    );
}

/// Verify XOR gate behavior with truth table
pub fn verify_xor_behavior(graph: &torg_core::Graph, output_id: u16) {
    use std::collections::HashMap;
    use torg_core::evaluate;

    // XOR truth table: T,F→T; F,T→T; F,F→F; T,T→F
    let inputs_map =
        |a: bool, b: bool| -> HashMap<u16, bool> { [(0, a), (1, b)].into_iter().collect() };

    assert!(
        evaluate(graph, &inputs_map(true, false)).unwrap()[&output_id],
        "XOR(T,F) should be true"
    );
    assert!(
        evaluate(graph, &inputs_map(false, true)).unwrap()[&output_id],
        "XOR(F,T) should be true"
    );
    assert!(
        !evaluate(graph, &inputs_map(false, false)).unwrap()[&output_id],
        "XOR(F,F) should be false"
    );
    assert!(
        !evaluate(graph, &inputs_map(true, true)).unwrap()[&output_id],
        "XOR(T,T) should be false"
    );
}
