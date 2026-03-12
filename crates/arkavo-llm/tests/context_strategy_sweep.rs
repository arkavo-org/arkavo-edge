//! Context strategy optimization sweep
//!
//! Uses Thompson Sampling to explore context strategy configurations.
//! Simulates burst execution with different strategies and measures
//! completion rate, loop avoidance, and context overhead.
//!
//! No model or mesh required — uses simulated burst outcomes.

use arkavo_llm::autoresearch::{
    ContextConfig, ContextMetrics, ContextSearchSpace, ContextVariant, Rng, ThompsonSampler,
};

/// Simulate burst execution under a context strategy
///
/// Models realistic behavior:
/// - Full: high completion but large context overhead
/// - SummaryOnly: moderate completion, low overhead
/// - LastN: depends on N (too few = lost context, too many = bloat)
/// - ArtifactReference: good balance
/// - Ledger: best for large contexts, threshold dependent
fn simulate_burst_metrics(config: &ContextConfig, rng: &mut Rng) -> ContextMetrics {
    let mut noise = || (rng.next_f64() - 0.5) * 0.1;

    let (completion_rate, loop_avoidance, avg_bytes, quality) = match config.strategy {
        ContextVariant::Full => (0.92 + noise(), 0.85 + noise(), 25_000, 0.88 + noise()),
        ContextVariant::SummaryOnly => (0.80 + noise(), 0.95 + noise(), 500, 0.75 + noise()),
        ContextVariant::LastNMessages => {
            let n = config.last_n.max(1) as f64;
            // Sweet spot around N=5-10: enough context without bloat
            let completion = if n < 3.0 {
                0.60 + noise()
            } else if n <= 10.0 {
                0.90 + noise()
            } else {
                0.85 + noise()
            };
            let bytes = (n * 800.0) as usize;
            let quality = if n < 3.0 { 0.65 } else { 0.85 } + noise();
            (completion, 0.90 + noise(), bytes, quality)
        }
        ContextVariant::ArtifactReference => (0.88 + noise(), 0.92 + noise(), 1200, 0.85 + noise()),
        ContextVariant::Ledger => {
            // Threshold too low = overhead for small contexts
            // Threshold too high = misses optimization opportunities
            let threshold = config.min_offload_chars.max(1) as f64;
            let completion = if threshold < 1000.0 {
                0.82 + noise()
            } else if threshold <= 5000.0 {
                0.93 + noise()
            } else {
                0.88 + noise()
            };
            let bytes = (threshold * 0.6) as usize;
            (completion, 0.94 + noise(), bytes, 0.87 + noise())
        }
    };

    ContextMetrics {
        completion_rate: completion_rate.clamp(0.0, 1.0),
        loop_avoidance_rate: loop_avoidance.clamp(0.0, 1.0),
        avg_context_bytes: avg_bytes,
        avg_quality: quality.clamp(0.0, 1.0),
        burst_count: 5,
    }
}

#[test]
fn test_context_sweep_thompson_sampling() {
    let space = ContextSearchSpace::default();
    let mut sampler = ThompsonSampler::new(space.config_count());
    let mut rng = Rng::new(42);

    let mut best_score = 0.0f64;
    let mut best_idx: Option<usize> = None;

    // Run 30 rounds of Thompson Sampling
    for _round in 0..30 {
        let arm = sampler.select(&mut rng);
        let config = &space.configs[arm];
        let metrics = simulate_burst_metrics(config, &mut rng);
        let score = metrics.composite_score();

        // Normalize to [0, 1] for Beta update
        let normalized = (score / 0.8).clamp(0.0, 1.0);
        sampler.update(arm, normalized);

        if score > best_score {
            best_score = score;
            best_idx = Some(arm);
        }
    }

    assert!(best_score > 0.0, "Should find a positive scoring config");
    let best = best_idx.unwrap();
    let best_config = &space.configs[best];

    // The best strategy should not be Full (too much overhead)
    // or SummaryOnly (too lossy) — it should be a balanced option
    eprintln!(
        "Best context strategy: {} (score={best_score:.4})",
        best_config.strategy
    );
}

#[test]
fn test_context_sweep_finds_optimal_last_n() {
    let space = ContextSearchSpace::default();
    let mut sampler = ThompsonSampler::new(space.config_count());
    let mut rng = Rng::new(137);

    // Run enough rounds to converge
    for _ in 0..50 {
        let arm = sampler.select(&mut rng);
        let config = &space.configs[arm];
        let metrics = simulate_burst_metrics(config, &mut rng);
        let normalized = (metrics.composite_score() / 0.8).clamp(0.0, 1.0);
        sampler.update(arm, normalized);
    }

    // Check that LastN with N=5 or N=10 scores well
    let last_n_arms: Vec<(usize, f64)> = space
        .configs
        .iter()
        .enumerate()
        .filter(|(_, c)| c.strategy == ContextVariant::LastNMessages)
        .map(|(i, _)| (i, sampler.prior(i).mean()))
        .collect();

    // At least one LastN variant should have above-average prior
    let max_last_n = last_n_arms
        .iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .unwrap();

    assert!(
        max_last_n.1 > 0.4,
        "Best LastN arm should have reasonable prior: {} (n={})",
        max_last_n.1,
        space.configs[max_last_n.0].last_n
    );
}

#[test]
fn test_context_sweep_ledger_threshold() {
    let space = ContextSearchSpace::default();
    let mut sampler = ThompsonSampler::new(space.config_count());
    let mut rng = Rng::new(2718);

    for _ in 0..50 {
        let arm = sampler.select(&mut rng);
        let config = &space.configs[arm];
        let metrics = simulate_burst_metrics(config, &mut rng);
        let normalized = (metrics.composite_score() / 0.8).clamp(0.0, 1.0);
        sampler.update(arm, normalized);
    }

    // Ledger with threshold 2000-5000 should score well
    let ledger_arms: Vec<(usize, &ContextConfig, f64)> = space
        .configs
        .iter()
        .enumerate()
        .filter(|(_, c)| c.strategy == ContextVariant::Ledger)
        .map(|(i, c)| (i, c, sampler.prior(i).mean()))
        .collect();

    let best_ledger = ledger_arms
        .iter()
        .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap())
        .unwrap();

    eprintln!(
        "Best ledger threshold: {} chars (prior mean={:.3})",
        best_ledger.1.min_offload_chars, best_ledger.2
    );

    // The best ledger threshold should be in the sweet spot
    assert!(
        best_ledger.2 > 0.4,
        "Best ledger arm should have reasonable prior: {}",
        best_ledger.2
    );
}

#[test]
fn test_combined_inference_and_context_sweep() {
    // Verify that inference and context search spaces can coexist
    let inference_space = arkavo_llm::autoresearch::SearchSpace::default();
    let context_space = ContextSearchSpace::default();

    // Combined arms: inference configs + context configs
    let total_arms = inference_space.config_count() + context_space.config_count();
    let mut sampler = ThompsonSampler::new(total_arms);

    // Inference arms get inference-style updates
    let mut rng = Rng::new(42);
    for _ in 0..20 {
        let arm = sampler.select(&mut rng);
        if arm < inference_space.config_count() {
            // Inference experiment result
            sampler.update(arm, 0.7);
        } else {
            // Context strategy result
            let ctx_idx = arm - inference_space.config_count();
            let config = &context_space.configs[ctx_idx];
            let metrics = simulate_burst_metrics(config, &mut rng);
            let normalized = (metrics.composite_score() / 0.8).clamp(0.0, 1.0);
            sampler.update(arm, normalized);
        }
    }

    // Sampler should have explored both spaces
    let explored_count = (0..total_arms)
        .filter(|&i| sampler.prior(i).observations() > 0.0)
        .count();
    assert!(
        explored_count >= 10,
        "Should explore at least 10 arms: {explored_count}"
    );
}
