//! Autoresearch Phase 2: Unified Parameter Sweep with Thompson Sampling
//!
//! Extends Phase 1's exhaustive grid search into intelligent exploration:
//! - Thompson Sampling (Beta priors) guides which configs to try next
//! - Seed variation breaks deterministic sampling to reveal temperature effects
//! - BurstContract-style per-experiment wall-time caps
//! - Weighted quality metric: relative_improvement × log(baseline_tok_per_sec)
//! - Kernel variant parameter (placeholder for precompiled MSL variants)
//!
//! Usage:
//!   ARKAVO_BENCH_MODEL=/path/to/qwen3.5-0.8b.gguf cargo test -p arkavo-llm --test autoresearch_burst --features llama-cpp -- --nocapture

#![cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#![allow(clippy::disallowed_methods)]

use arkavo_llama_cpp::{
    LlamaContext, LlamaModel, batch_get_one, batch_get_one_with_logits, create_sampler_chain,
    decode_batch, init_llama_logging, perf_context_reset, token_to_piece, tokenize_with_model,
};
use arkavo_llm::autoresearch::{BetaPrior, Rng, evaluate_quality};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Experiment parameter space — each dimension is an arm in Thompson Sampling
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct ParamConfig {
    enable_thinking: bool,
    temperature_idx: usize, // index into TEMPERATURES
    max_tokens_idx: usize,  // index into MAX_TOKENS
    top_p_idx: usize,       // index into TOP_P_VALUES
    kernel_variant: u32,    // 0=baseline (future: precompiled MSL variants)
}

const TEMPERATURES: &[f32] = &[0.1, 0.3, 0.5, 0.7, 0.9, 1.2];
const MAX_TOKENS: &[u32] = &[32, 64, 128, 256];
const TOP_P_VALUES: &[f32] = &[0.8, 0.9, 0.95, 1.0];
const KERNEL_VARIANTS: u32 = 1; // expand when precompiled MSL variants exist
const SEEDS: &[u32] = &[42, 137, 2718, 31415, 65537];

/// Per-experiment wall-time cap (BurstContract.max_wall_time equivalent)
const EXPERIMENT_TIMEOUT: Duration = Duration::from_secs(10);

/// Evaluation prompts — same as Phase 1 for comparability
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

/// Run inference with given config and seed, respecting per-experiment timeout
fn run_experiment(
    model: &LlamaModel,
    config: &ParamConfig,
    seed: u32,
    prompt: &str,
) -> Option<(f64, f64, u32, String)> {
    let mut ctx = LlamaContext::new(model).ok()?;
    perf_context_reset(&mut ctx);

    let vocab = model.get_vocab();
    let temperature = TEMPERATURES[config.temperature_idx];
    let max_tokens = MAX_TOKENS[config.max_tokens_idx];
    let top_p = TOP_P_VALUES[config.top_p_idx];

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

    let prompt_start = Instant::now();
    let batch = batch_get_one_with_logits(&tokens, true);
    decode_batch(&ctx, batch).ok()?;
    let ttft_ms = prompt_start.elapsed().as_secs_f64() * 1000.0;

    let sampler = create_sampler_chain(temperature, top_p, 40, seed).ok()?;

    let eos = model.get_eos_token();
    let mut generated_text = String::new();
    let mut tokens_generated = 0u32;
    let gen_start = Instant::now();

    for _ in 0..max_tokens {
        // BurstContract-style per-experiment timeout
        if gen_start.elapsed() >= EXPERIMENT_TIMEOUT {
            break;
        }

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

    let gen_elapsed = gen_start.elapsed().as_secs_f64();
    let tok_per_sec = if gen_elapsed > 0.0 && tokens_generated > 0 {
        tokens_generated as f64 / gen_elapsed
    } else {
        0.0
    };

    Some((tok_per_sec, ttft_ms, tokens_generated, generated_text))
}

/// Build all possible configs (arms for Thompson Sampling)
fn build_all_configs() -> Vec<ParamConfig> {
    let mut configs = Vec::new();
    for thinking in [false, true] {
        for t_idx in 0..TEMPERATURES.len() {
            for m_idx in 0..MAX_TOKENS.len() {
                for p_idx in 0..TOP_P_VALUES.len() {
                    for kv in 0..KERNEL_VARIANTS {
                        configs.push(ParamConfig {
                            enable_thinking: thinking,
                            temperature_idx: t_idx,
                            max_tokens_idx: m_idx,
                            top_p_idx: p_idx,
                            kernel_variant: kv,
                        });
                    }
                }
            }
        }
    }
    configs
}

/// Select next config using Thompson Sampling
fn thompson_select(
    configs: &[ParamConfig],
    priors: &HashMap<usize, BetaPrior>,
    rng: &mut Rng,
) -> usize {
    configs
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let prior = priors.get(&i).cloned().unwrap_or_else(BetaPrior::new);
            (i, prior.sample(rng))
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Weighted quality metric: quality × log(1 + tok_per_sec)
/// Rewards high throughput — a config running at 200 tok/s with quality 0.8
/// beats one at 100 tok/s with quality 0.85
fn weighted_quality(quality_per_token: f64, tok_per_sec: f64) -> f64 {
    quality_per_token * tok_per_sec.ln_1p()
}

#[test]
fn autoresearch_burst() {
    let model_path = match std::env::var("ARKAVO_BENCH_MODEL") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("ARKAVO_BENCH_MODEL not set, skipping autoresearch burst");
            eprintln!(
                "Usage: ARKAVO_BENCH_MODEL=/path/to/qwen3.5-0.8b.gguf cargo test -p arkavo-llm --test autoresearch_burst --features llama-cpp -- --nocapture"
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

    let total_budget = Duration::from_secs(
        std::env::var("ARKAVO_SWEEP_BUDGET_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300),
    );

    let results_path = PathBuf::from(
        std::env::var("ARKAVO_SWEEP_RESULTS")
            .unwrap_or_else(|_| "autoresearch_burst_results.tsv".to_string()),
    );

    // Write TSV header
    {
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&results_path)
            .expect("Failed to create results file");
        writeln!(
            f,
            "round\tthinking\ttemperature\tmax_tokens\ttop_p\tkernel_variant\tseed\ttok_per_sec\tttft_ms\ttokens_generated\tquality_score\tquality_per_token\tweighted_quality\twall_time_ms\tprompt_category"
        )
        .expect("Failed to write header");
    }

    let configs = build_all_configs();
    let mut priors: HashMap<usize, BetaPrior> = HashMap::new();
    let mut rng = Rng::new(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(12345),
    );

    let max_rounds: u32 = std::env::var("ARKAVO_SWEEP_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    let sweep_start = Instant::now();
    let mut best_weighted = 0.0f64;
    let mut best_config_idx: Option<usize> = None;
    let mut experiment_count = 0u32;
    let mut kept_count = 0u32;

    // Establish baseline with first config (thinking=false, temp=0.1, max_tok=32)
    let mut baseline_tok_per_sec = 0.0f64;

    eprintln!("\nAutoresearch Phase 2: Thompson Sampling Burst");
    eprintln!(
        "Configs: {} | Rounds: {} | Seeds: {} | Budget: {}s",
        configs.len(),
        max_rounds,
        SEEDS.len(),
        total_budget.as_secs()
    );
    eprintln!("Results: {}\n", results_path.display());

    for round in 0..max_rounds {
        if sweep_start.elapsed() >= total_budget {
            eprintln!("\nBudget exhausted after {experiment_count} experiments");
            break;
        }

        // Thompson Sampling selects the next config to try
        let config_idx = thompson_select(&configs, &priors, &mut rng);
        let config = &configs[config_idx];

        let mut round_quality_sum = 0.0f64;
        let mut round_tok_per_sec_sum = 0.0f64;
        let mut round_count = 0u32;

        // Run across multiple seeds and prompts
        for &seed in SEEDS {
            if sweep_start.elapsed() >= total_budget {
                break;
            }

            for (category, prompt) in EVAL_PROMPTS {
                if sweep_start.elapsed() >= total_budget {
                    break;
                }

                let exp_start = Instant::now();

                let (tok_per_sec, ttft_ms, tokens_generated, text) =
                    match run_experiment(&model, config, seed, prompt) {
                        Some(r) => r,
                        None => continue,
                    };

                // Set baseline from first successful experiment
                if baseline_tok_per_sec < f64::EPSILON {
                    baseline_tok_per_sec = tok_per_sec;
                }

                let quality_score = evaluate_quality(&text);
                let quality_per_token = if tokens_generated > 0 {
                    quality_score / tokens_generated as f64
                } else {
                    0.0
                };
                let wq = weighted_quality(quality_per_token, tok_per_sec);
                let wall_time_ms = exp_start.elapsed().as_millis() as u64;

                // Log to TSV
                let mut f = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&results_path)
                    .unwrap();
                let _ = writeln!(
                    f,
                    "{round}\t{}\t{:.2}\t{}\t{:.2}\t{}\t{seed}\t{tok_per_sec:.1}\t{ttft_ms:.1}\t{tokens_generated}\t{quality_score:.3}\t{quality_per_token:.6}\t{wq:.6}\t{wall_time_ms}\t{category}",
                    config.enable_thinking,
                    TEMPERATURES[config.temperature_idx],
                    MAX_TOKENS[config.max_tokens_idx],
                    TOP_P_VALUES[config.top_p_idx],
                    config.kernel_variant,
                );

                round_quality_sum += wq;
                round_tok_per_sec_sum += tok_per_sec;
                round_count += 1;
                experiment_count += 1;
            }
        }

        if round_count == 0 {
            continue;
        }

        let avg_weighted = round_quality_sum / round_count as f64;
        let avg_tok_per_sec = round_tok_per_sec_sum / round_count as f64;

        // Update Thompson Sampling prior for this config
        let prior = priors.entry(config_idx).or_default();
        // Normalize weighted quality to [0, 1] for Beta update
        let normalized = (avg_weighted / 0.1).clamp(0.0, 1.0);
        prior.update(normalized);

        let is_improvement = avg_weighted > best_weighted;
        let status = if is_improvement {
            best_weighted = avg_weighted;
            best_config_idx = Some(config_idx);
            kept_count += 1;
            "KEEP"
        } else {
            "REVERT"
        };

        eprintln!(
            "[R{round:>3}] thinking={:<5} temp={:.1} max_tok={:<3} top_p={:.2} kv={} → wq={:.6} tok/s={:.1} prior={:.3} [{status}]",
            config.enable_thinking,
            TEMPERATURES[config.temperature_idx],
            MAX_TOKENS[config.max_tokens_idx],
            TOP_P_VALUES[config.top_p_idx],
            config.kernel_variant,
            avg_weighted,
            avg_tok_per_sec,
            prior.mean(),
        );
    }

    // Summary
    let total_time = sweep_start.elapsed();
    eprintln!("\n========================================");
    eprintln!("Autoresearch Phase 2 Complete");
    eprintln!("========================================");
    eprintln!("Total experiments:  {experiment_count}");
    eprintln!(
        "Rounds completed:   {}",
        kept_count
            + (experiment_count / SEEDS.len() as u32 / EVAL_PROMPTS.len() as u32)
                .saturating_sub(kept_count)
    );
    eprintln!("Kept improvements:  {kept_count}");
    eprintln!("Baseline tok/s:     {baseline_tok_per_sec:.1}");
    eprintln!("Total wall time:    {:.1}s", total_time.as_secs_f64());
    eprintln!(
        "Experiments/hour:   {:.0}",
        experiment_count as f64 / total_time.as_secs_f64() * 3600.0
    );
    eprintln!("Results file:       {}", results_path.display());

    if let Some(idx) = best_config_idx {
        let best = &configs[idx];
        eprintln!("\nBest configuration:");
        eprintln!("  enable_thinking:  {}", best.enable_thinking);
        eprintln!(
            "  temperature:      {:.2}",
            TEMPERATURES[best.temperature_idx]
        );
        eprintln!("  max_tokens:       {}", MAX_TOKENS[best.max_tokens_idx]);
        eprintln!("  top_p:            {:.2}", TOP_P_VALUES[best.top_p_idx]);
        eprintln!("  kernel_variant:   {}", best.kernel_variant);
        eprintln!("  weighted_quality: {best_weighted:.6}");

        let toml_path = results_path.with_extension("toml");
        if let Ok(mut f) = std::fs::File::create(&toml_path) {
            let _ = writeln!(
                f,
                "# Autoresearch Phase 2: Best configuration (Thompson Sampling)"
            );
            let _ = writeln!(
                f,
                "# Generated on {}",
                chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
            );
            let _ = writeln!(f, "# weighted_quality = {best_weighted:.6}");
            let _ = writeln!(f, "# baseline_tok_per_sec = {baseline_tok_per_sec:.1}");
            let prior = priors.get(&idx).cloned().unwrap_or_else(BetaPrior::new);
            let _ = writeln!(f, "# prior_mean = {:.4}", prior.mean());
            let _ = writeln!(f);
            let _ = writeln!(f, "[inference]");
            let _ = writeln!(f, "enable_thinking = {}", best.enable_thinking);
            let _ = writeln!(f, "temperature = {:.2}", TEMPERATURES[best.temperature_idx]);
            let _ = writeln!(f, "max_tokens = {}", MAX_TOKENS[best.max_tokens_idx]);
            let _ = writeln!(f, "top_p = {:.2}", TOP_P_VALUES[best.top_p_idx]);
            let _ = writeln!(f);
            let _ = writeln!(f, "[kernel]");
            let _ = writeln!(f, "variant = {}", best.kernel_variant);
            eprintln!("  Config written to: {}", toml_path.display());
        }
    }

    assert!(
        experiment_count > 0 || std::env::var("ARKAVO_BENCH_MODEL").is_err(),
        "Expected at least 1 experiment to complete"
    );
}

#[cfg(test)]
mod thompson_tests {
    use super::*;

    #[test]
    fn test_beta_prior_initial() {
        let p = BetaPrior::new();
        assert!((p.mean() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_beta_prior_update_success() {
        let mut p = BetaPrior::new();
        p.update(1.0);
        // alpha=2, beta=1 → mean = 2/3
        assert!((p.mean() - 2.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn test_beta_prior_update_failure() {
        let mut p = BetaPrior::new();
        p.update(0.0);
        // alpha=1, beta=2 → mean = 1/3
        assert!((p.mean() - 1.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn test_beta_prior_convergence() {
        let mut p = BetaPrior::new();
        for _ in 0..100 {
            p.update(0.8);
        }
        // Should converge near 0.8
        assert!((p.mean() - 0.8).abs() < 0.05, "mean={}", p.mean());
    }

    #[test]
    fn test_thompson_sampling_selects() {
        let configs = build_all_configs();
        let priors = HashMap::new();
        let mut rng = Rng::new(42);
        let idx = thompson_select(&configs, &priors, &mut rng);
        assert!(idx < configs.len());
    }

    #[test]
    fn test_thompson_prefers_better_arm() {
        let configs = build_all_configs();
        let mut priors = HashMap::new();

        // Give all arms poor priors so they don't compete
        for i in 0..configs.len() {
            let mut bad = BetaPrior::new();
            for _ in 0..20 {
                bad.update(0.1);
            }
            priors.insert(i, bad);
        }

        // Override arm 5 with many successes
        let mut good = BetaPrior::new();
        for _ in 0..50 {
            good.update(0.9);
        }
        priors.insert(5, good);

        // Over many selections, arm 5 should dominate
        let mut rng = Rng::new(42);
        let mut count_5 = 0;
        for _ in 0..100 {
            let idx = thompson_select(&configs, &priors, &mut rng);
            if idx == 5 {
                count_5 += 1;
            }
        }
        assert!(
            count_5 > 30,
            "Arm 5 should be selected frequently, got {count_5}/100"
        );
    }

    #[test]
    fn test_weighted_quality_metric() {
        // Higher tok/s should boost the metric
        let wq_fast = weighted_quality(0.01, 200.0);
        let wq_slow = weighted_quality(0.01, 50.0);
        assert!(wq_fast > wq_slow, "fast={wq_fast} > slow={wq_slow}");
    }

    #[test]
    fn test_weighted_quality_zero() {
        let wq = weighted_quality(0.0, 100.0);
        assert!(wq.abs() < f64::EPSILON);
    }

    #[test]
    fn test_simple_rng_deterministic() {
        let mut r1 = Rng::new(42);
        let mut r2 = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(r1.next_u64(), r2.next_u64());
        }
    }

    #[test]
    fn test_simple_rng_range() {
        let mut rng = Rng::new(42);
        for _ in 0..1000 {
            let v = rng.next_f64();
            assert!((0.0..1.0).contains(&v), "out of range: {v}");
        }
    }

    #[test]
    fn test_config_space_size() {
        let configs = build_all_configs();
        let expected = 2
            * TEMPERATURES.len()
            * MAX_TOKENS.len()
            * TOP_P_VALUES.len()
            * KERNEL_VARIANTS as usize;
        assert_eq!(configs.len(), expected);
    }
}
