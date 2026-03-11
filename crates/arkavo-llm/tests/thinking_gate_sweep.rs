//! Autoresearch Phase 1: Thinking Gate Parameter Sweep
//!
//! Implements the autoresearch loop for the thinking gate optimization:
//! 1. Read config — load experiment parameters
//! 2. Mutate — apply one parameter change from the search space
//! 3. Execute — run inference with the parameter configuration
//! 4. Evaluate — measure quality_per_token via CriticPipeline + perf_context
//! 5. Decide — keep/revert based on metric improvement
//! 6. Log — append results to TSV for analysis
//! 7. Loop — repeat until wall-clock budget exhausted
//!
//! Requires the `llama-cpp` feature and a Qwen3.5 GGUF model file.
//! Set ARKAVO_BENCH_MODEL to the model path, or the sweep will skip.
//!
//! Usage:
//!   ARKAVO_BENCH_MODEL=/path/to/qwen3.5-0.8b.gguf cargo test -p arkavo-llm --test thinking_gate_sweep --features llama-cpp -- --nocapture

#![cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#![allow(clippy::disallowed_methods)]

use arkavo_llama_cpp::{
    LlamaContext, LlamaModel, batch_get_one, batch_get_one_with_logits, create_sampler_chain,
    decode_batch, init_llama_logging, perf_context, perf_context_reset, token_to_piece,
    tokenize_with_model,
};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// A single experiment configuration
#[derive(Debug, Clone)]
struct ExperimentConfig {
    /// Whether thinking/CoT is enabled
    enable_thinking: bool,
    /// Sampling temperature
    temperature: f32,
    /// Maximum tokens to generate
    max_tokens: u32,
    /// Top-p nucleus sampling parameter
    top_p: f32,
    /// Top-k sampling parameter
    top_k: i32,
}

/// Result of a single experiment run
#[derive(Debug, Clone)]
struct ExperimentResult {
    config: ExperimentConfig,
    /// Tokens per second during generation
    tok_per_sec: f64,
    /// Time to first token in milliseconds
    ttft_ms: f64,
    /// Total tokens generated
    tokens_generated: u32,
    /// Quality score: 1.0 if response passes lint checks, 0.0 if not
    quality_score: f64,
    /// Composite metric: quality / tokens (rewards efficiency)
    quality_per_token: f64,
    /// Wall-clock time for this experiment
    wall_time_ms: u64,
}

/// Evaluation prompts — fixed across all experiments (the prepare.py equivalent)
const EVAL_PROMPTS: &[(&str, &str)] = &[
    (
        "factual",
        "What is the capital of France? Answer in one sentence.",
    ),
    (
        "reasoning",
        "If a train travels 60 km/h for 2.5 hours, how far does it go? Show your work.",
    ),
    (
        "coding",
        "Write a Python function that checks if a string is a palindrome.",
    ),
    ("creative", "Write a haiku about the ocean."),
    (
        "instruction",
        "List three benefits of exercise. Be concise.",
    ),
];

/// Simple quality evaluator (inline Critic checks without full pipeline dependency)
/// Checks for: non-empty, no truncation, no excessive repetition, no placeholder text
fn evaluate_quality(response: &str) -> f64 {
    let trimmed = response.trim();

    // Empty response: total failure
    if trimmed.is_empty() {
        return 0.0;
    }

    let mut score: f64 = 1.0;

    // Truncation: unclosed code blocks
    let backtick_count = trimmed.matches("```").count();
    if !backtick_count.is_multiple_of(2) {
        score -= 0.3;
    }

    // Repetition: unique word ratio
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.len() >= 10 {
        let unique: std::collections::HashSet<&str> = words.iter().copied().collect();
        let ratio = unique.len() as f64 / words.len() as f64;
        if ratio < 0.3 {
            score -= 0.5; // Severe repetition
        } else if ratio < 0.5 {
            score -= 0.2; // Moderate repetition
        }
    }

    // Placeholder / failure patterns
    let lower = trimmed.to_lowercase();
    let failure_patterns = [
        "as an ai language model",
        "as a large language model",
        "[todo",
        "[placeholder",
        "lorem ipsum",
    ];
    for pattern in failure_patterns {
        if lower.contains(pattern) {
            score -= 0.3;
        }
    }

    // Very short response for non-trivial prompts
    if words.len() < 3 {
        score -= 0.2;
    }

    score.clamp(0.0, 1.0)
}

/// Run a single inference experiment and measure metrics
fn run_experiment(
    model: &LlamaModel,
    config: &ExperimentConfig,
    prompt: &str,
) -> Option<(f64, f64, u32, String)> {
    let mut ctx = LlamaContext::new(model).ok()?;
    perf_context_reset(&mut ctx);

    let vocab = model.get_vocab();

    // Build prompt with or without thinking instruction
    let full_prompt = if config.enable_thinking {
        format!(
            "<|im_start|>system\nThink step by step before answering.<|im_end|>\n<|im_start|>user\n{prompt}<|im_end|>\n<|im_start|>assistant\n"
        )
    } else {
        format!(
            "<|im_start|>system\nYou are a helpful assistant. Be concise.<|im_end|>\n<|im_start|>user\n{prompt}<|im_end|>\n<|im_start|>assistant\n"
        )
    };

    let tokens = tokenize_with_model(vocab, full_prompt.as_bytes()).ok()?;

    // Prompt eval
    let batch = batch_get_one_with_logits(&tokens, true);
    decode_batch(&ctx, batch).ok()?;

    // Create sampler with experiment config
    let sampler = create_sampler_chain(config.temperature, config.top_p, config.top_k, 42).ok()?;

    let eos = model.get_eos_token();
    let mut generated_text = String::new();
    let mut tokens_generated = 0u32;

    for _ in 0..config.max_tokens {
        let token = sampler.sample(&ctx, -1);
        if token == eos {
            break;
        }

        if let Ok(piece) = token_to_piece(vocab, token, false) {
            generated_text.push_str(&piece);
        }

        let next_batch = batch_get_one(&[token]);
        if decode_batch(&ctx, next_batch).is_err() {
            break;
        }
        tokens_generated += 1;
    }

    let metrics = perf_context(&ctx);
    Some((
        metrics.tok_per_sec(),
        metrics.ttft_ms(),
        tokens_generated,
        generated_text,
    ))
}

/// The search space: all configurations to sweep
fn build_search_space() -> Vec<ExperimentConfig> {
    let mut configs = Vec::new();

    // Sweep thinking gate × temperature
    for enable_thinking in [false, true] {
        for &temperature in &[0.1, 0.3, 0.5, 0.7, 1.0] {
            for &max_tokens in &[64, 128, 256] {
                configs.push(ExperimentConfig {
                    enable_thinking,
                    temperature,
                    max_tokens,
                    top_p: 0.9,
                    top_k: 40,
                });
            }
        }
    }

    configs
}

/// Write results header to TSV file
fn write_header(path: &PathBuf) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    writeln!(
        file,
        "thinking\ttemperature\tmax_tokens\ttop_p\ttop_k\ttok_per_sec\tttft_ms\ttokens_generated\tquality_score\tquality_per_token\twall_time_ms\tprompt_category"
    )?;
    Ok(())
}

/// Append a result row to TSV file
fn write_result(
    path: &PathBuf,
    result: &ExperimentResult,
    prompt_category: &str,
) -> std::io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(
        file,
        "{}\t{:.2}\t{}\t{:.2}\t{}\t{:.1}\t{:.1}\t{}\t{:.3}\t{:.6}\t{}\t{}",
        result.config.enable_thinking,
        result.config.temperature,
        result.config.max_tokens,
        result.config.top_p,
        result.config.top_k,
        result.tok_per_sec,
        result.ttft_ms,
        result.tokens_generated,
        result.quality_score,
        result.quality_per_token,
        result.wall_time_ms,
        prompt_category,
    )?;
    Ok(())
}

#[test]
fn thinking_gate_sweep() {
    let model_path = match std::env::var("ARKAVO_BENCH_MODEL") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("ARKAVO_BENCH_MODEL not set, skipping thinking gate sweep");
            eprintln!(
                "Usage: ARKAVO_BENCH_MODEL=/path/to/qwen3.5-0.8b.gguf cargo test -p arkavo-llm --test thinking_gate_sweep --features llama-cpp -- --nocapture"
            );
            return;
        }
    };

    init_llama_logging();

    let model = match LlamaModel::from_file(&model_path) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Failed to load model: {e}");
            return;
        }
    };

    eprintln!("Loaded model: {model_path}");

    // Wall-clock budget for the entire sweep
    let total_budget = Duration::from_secs(
        std::env::var("ARKAVO_SWEEP_BUDGET_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300), // 5 minutes default
    );

    let results_path = PathBuf::from(
        std::env::var("ARKAVO_SWEEP_RESULTS")
            .unwrap_or_else(|_| "thinking_gate_results.tsv".to_string()),
    );

    write_header(&results_path).expect("Failed to write results header");

    let search_space = build_search_space();
    let sweep_start = Instant::now();
    let mut best_quality_per_token = 0.0f64;
    let mut best_config: Option<ExperimentConfig> = None;
    let mut experiment_count = 0u32;
    let mut kept_count = 0u32;

    eprintln!(
        "\nStarting sweep: {} configs × {} prompts = {} experiments (budget: {}s)",
        search_space.len(),
        EVAL_PROMPTS.len(),
        search_space.len() * EVAL_PROMPTS.len(),
        total_budget.as_secs()
    );
    eprintln!("Results: {}\n", results_path.display());

    for config in &search_space {
        // Check wall-clock budget
        if sweep_start.elapsed() >= total_budget {
            eprintln!("\nBudget exhausted after {experiment_count} experiments");
            break;
        }

        let mut config_quality_sum = 0.0f64;
        let mut config_tok_per_sec_sum = 0.0f64;
        let mut config_prompt_count = 0u32;

        for (category, prompt) in EVAL_PROMPTS {
            if sweep_start.elapsed() >= total_budget {
                break;
            }

            let exp_start = Instant::now();

            let (tok_per_sec, ttft_ms, tokens_generated, generated_text) =
                match run_experiment(&model, config, prompt) {
                    Some(r) => r,
                    None => {
                        eprintln!("  [SKIP] Experiment failed for {category}");
                        continue;
                    }
                };

            let quality_score = evaluate_quality(&generated_text);

            // Composite metric: quality normalized by token cost
            // A response that gets quality 0.9 in 50 tokens beats 0.95 in 200 tokens
            let quality_per_token = if tokens_generated > 0 {
                quality_score / tokens_generated as f64
            } else {
                0.0
            };

            let wall_time_ms = exp_start.elapsed().as_millis() as u64;

            let result = ExperimentResult {
                config: config.clone(),
                tok_per_sec,
                ttft_ms,
                tokens_generated,
                quality_score,
                quality_per_token,
                wall_time_ms,
            };

            let _ = write_result(&results_path, &result, category);

            config_quality_sum += quality_per_token;
            config_tok_per_sec_sum += tok_per_sec;
            config_prompt_count += 1;
            experiment_count += 1;
        }

        if config_prompt_count == 0 {
            continue;
        }

        // Average quality_per_token across all prompts for this config
        let avg_quality_per_token = config_quality_sum / config_prompt_count as f64;
        let avg_tok_per_sec = config_tok_per_sec_sum / config_prompt_count as f64;

        // Keep/revert decision
        let is_improvement = avg_quality_per_token > best_quality_per_token;

        let status = if is_improvement {
            best_quality_per_token = avg_quality_per_token;
            best_config = Some(config.clone());
            kept_count += 1;
            "KEEP"
        } else {
            "REVERT"
        };

        eprintln!(
            "[{:>3}] thinking={:<5} temp={:.1} max_tok={:<3} → q/tok={:.6} tok/s={:.1} [{status}]",
            experiment_count,
            config.enable_thinking,
            config.temperature,
            config.max_tokens,
            avg_quality_per_token,
            avg_tok_per_sec,
        );
    }

    // Summary
    let total_time = sweep_start.elapsed();
    eprintln!("\n========================================");
    eprintln!("Thinking Gate Sweep Complete");
    eprintln!("========================================");
    eprintln!("Total experiments:  {experiment_count}");
    eprintln!("Kept improvements:  {kept_count}");
    eprintln!("Total wall time:    {:.1}s", total_time.as_secs_f64());
    eprintln!(
        "Experiments/hour:   {:.0}",
        experiment_count as f64 / total_time.as_secs_f64() * 3600.0
    );
    eprintln!("Results file:       {}", results_path.display());

    if let Some(best) = &best_config {
        eprintln!("\nBest configuration:");
        eprintln!("  enable_thinking:  {}", best.enable_thinking);
        eprintln!("  temperature:      {:.2}", best.temperature);
        eprintln!("  max_tokens:       {}", best.max_tokens);
        eprintln!("  top_p:            {:.2}", best.top_p);
        eprintln!("  top_k:            {}", best.top_k);
        eprintln!("  quality_per_token: {best_quality_per_token:.6}");

        // Write winning config to TOML
        let toml_path = results_path.with_extension("toml");
        if let Ok(mut f) = std::fs::File::create(&toml_path) {
            let _ = writeln!(
                f,
                "# Autoresearch Phase 1: Best thinking gate configuration"
            );
            let _ = writeln!(
                f,
                "# Generated by thinking_gate_sweep on {}",
                chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
            );
            let _ = writeln!(f, "# quality_per_token = {best_quality_per_token:.6}");
            let _ = writeln!(f);
            let _ = writeln!(f, "[thinking_gate]");
            let _ = writeln!(f, "enable_thinking = {}", best.enable_thinking);
            let _ = writeln!(f, "temperature = {:.2}", best.temperature);
            let _ = writeln!(f, "max_tokens = {}", best.max_tokens);
            let _ = writeln!(f, "top_p = {:.2}", best.top_p);
            let _ = writeln!(f, "top_k = {}", best.top_k);
            eprintln!("  Config written to: {}", toml_path.display());
        }
    } else {
        eprintln!("\nNo valid experiments completed.");
    }

    // Assert at least some experiments ran (guard against silent failures)
    assert!(
        experiment_count > 0 || std::env::var("ARKAVO_BENCH_MODEL").is_err(),
        "Expected at least 1 experiment to complete"
    );
}

#[cfg(test)]
mod quality_tests {
    use super::*;

    #[test]
    fn test_evaluate_quality_good_response() {
        let score = evaluate_quality("The capital of France is Paris.");
        assert!(score > 0.8, "Good response should score high: {score}");
    }

    #[test]
    fn test_evaluate_quality_empty() {
        let score = evaluate_quality("");
        assert!(
            score < f64::EPSILON,
            "Empty response should score 0: {score}"
        );
    }

    #[test]
    fn test_evaluate_quality_repetitive() {
        let repeated = "the the the the the the the the the the the the the the the";
        let score = evaluate_quality(repeated);
        assert!(
            score <= 0.5,
            "Repetitive response should score low: {score}"
        );
    }

    #[test]
    fn test_evaluate_quality_truncated_code() {
        let truncated = "Here's the code:\n```python\ndef foo():";
        let score = evaluate_quality(truncated);
        assert!(score < 0.8, "Truncated code should lose points: {score}");
    }

    #[test]
    fn test_evaluate_quality_placeholder() {
        let placeholder = "Here is the answer: [TODO: implement this]";
        let score = evaluate_quality(placeholder);
        assert!(score < 0.8, "Placeholder should lose points: {score}");
    }

    #[test]
    fn test_evaluate_quality_ai_meta() {
        let meta = "As an AI language model, I cannot help with that request.";
        let score = evaluate_quality(meta);
        assert!(
            score < 0.8,
            "AI meta-commentary should lose points: {score}"
        );
    }
}
