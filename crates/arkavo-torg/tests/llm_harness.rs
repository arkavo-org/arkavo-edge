//! Test harness for LLM-based TØR-G integration tests
//!
//! Provides model loading, prompt formatting, and constrained generation utilities.

#![cfg(not(target_env = "musl"))]
#![allow(dead_code)] // Harness functions may not all be used in every test
#![allow(unreachable_pub)] // Test module visibility doesn't matter
#![allow(clippy::missing_panics_doc)] // Test harness doesn't need panic docs
#![allow(clippy::cast_possible_wrap)] // Token counts won't exceed i32

use std::path::PathBuf;

use arkavo_llama_cpp::{
    batch_free, batch_init_with_tokens, decode_batch, tokenize_with_model, LlamaContext,
    LlamaModel, LlamaSampler,
};
use arkavo_torg::{Qwen3TokenMap, TorgError, TorgLlamaSampler};
use torg_core::Graph;

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

/// System prompt for TØR-G generation
pub const TORG_SYSTEM_PROMPT: &str = r#"You are a TØR-G policy generator.
Output format: IN:id IN:id ... [node_id op arg1 arg2] ... OUT:node_id
Operators: | (OR), ! (NOR), ^ (XOR)
Example: IN:0 IN:1 [2 | 0 1] OUT:2 means "input0 OR input1"
Generate ONLY the TØR-G tokens, no explanation."#;

/// Format a policy description using Qwen3 chat template
pub fn format_prompt(policy: &str) -> String {
    format!(
        "<|im_start|>system\n{TORG_SYSTEM_PROMPT}<|im_end|>\n<|im_start|>user\n{policy}<|im_end|>\n<|im_start|>assistant\n",
    )
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

    // Build token mapping from vocabulary
    let mapping = unsafe { Qwen3TokenMap::from_vocab(vocab)? };
    let vocab_size = mapping.vocab_size();
    let mut torg_sampler = TorgLlamaSampler::new(mapping.into_mapping(), vocab_size);

    // Format and tokenize prompt
    let prompt = format_prompt(policy);
    let tokens = tokenize_with_model(vocab, prompt.as_bytes())
        .map_err(GenerationError::Tokenization)?;

    // Process prompt through model
    let mut batch = batch_init_with_tokens(&tokens, 0, true);
    decode_batch(ctx, batch).map_err(GenerationError::Decode)?;
    batch_free(&mut batch);

    let mut pos = tokens.len() as i32;
    let eos = model.get_eos_token();

    // Generation loop with constrained sampling
    for _ in 0..max_tokens {
        // Create fresh sampler chain for this token
        let llama_sampler = LlamaSampler::new_chain(true)
            .map_err(GenerationError::Sampler)?;
        llama_sampler.add_temp(0.0); // Greedy for determinism

        // Apply TØR-G constraints
        let bias = torg_sampler.get_logit_bias();
        llama_sampler.add_logit_bias(torg_sampler.vocab_size(), &bias);
        llama_sampler.add_greedy();

        // Sample next token
        let token = llama_sampler.sample(ctx, -1);

        // Check for completion
        if token == eos || torg_sampler.is_complete() {
            break;
        }

        // Feed token to TØR-G state machine
        torg_sampler.feed_token(token as u32)?;

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
    let inputs_map = |a: bool, b: bool| -> HashMap<u16, bool> {
        [(0, a), (1, b)].into_iter().collect()
    };

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
    let inputs_map = |a: bool, b: bool| -> HashMap<u16, bool> {
        [(0, a), (1, b)].into_iter().collect()
    };

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
