//! Integration tests for the autoresearch runtime subsystem
//!
//! Tests the full loop: experiment config → quality evaluation → Thompson update
//! → gossip announcement → remote merge → selection shift.
//! No model required — uses simulated experiment results.

use arkavo_gossip::experiment_message::{
    ExperimentAnnouncement, ExperimentMetrics, ExperimentVote, HardwareTier,
};
use arkavo_llm::autoresearch::{
    BetaPrior, ExperimentConfig, ExperimentResult, SearchSpace, ThompsonSampler, evaluate_quality,
};
use std::collections::HashMap;
use std::time::Duration;

/// Simulate running an experiment and producing a result
fn simulate_experiment(
    space: &SearchSpace,
    config: &ExperimentConfig,
    seed: u32,
    response: &str,
) -> ExperimentResult {
    let quality_score = evaluate_quality(response);
    let tokens_generated = response.split_whitespace().count() as u32;
    let quality_per_token = if tokens_generated > 0 {
        quality_score / tokens_generated as f64
    } else {
        0.0
    };
    // Simulate ~175 tok/s baseline
    let tok_per_sec = 170.0 + (seed % 20) as f64;
    let weighted_quality = ExperimentResult::compute_weighted(quality_per_token, tok_per_sec);
    let (temperature, _max_tokens, _top_p) = space.resolve(config);

    ExperimentResult {
        config: config.clone(),
        seed,
        tok_per_sec,
        ttft_ms: (temperature as f64).mul_add(2.0, 15.0),
        tokens_generated,
        quality_score,
        quality_per_token,
        weighted_quality,
        wall_time: Duration::from_millis((tokens_generated as f64 / tok_per_sec * 1000.0) as u64),
        prompt_category: "test".into(),
    }
}

/// Convert an experiment result into a gossip announcement
fn result_to_announcement(
    result: &ExperimentResult,
    prior: &BetaPrior,
    originator: &str,
    tier: HardwareTier,
) -> ExperimentAnnouncement {
    let config_json = serde_json::to_string(&result.config).unwrap();
    ExperimentAnnouncement::new(
        uuid::Uuid::new_v4(),
        [0u8; 32],
        originator.into(),
        tier,
        config_json,
        ExperimentMetrics {
            weighted_quality: result.weighted_quality,
            baseline_tok_per_sec: result.tok_per_sec,
            kept: result.weighted_quality > 0.15,
        },
    )
    .with_prior(prior.alpha, prior.beta)
}

#[test]
fn test_full_autoresearch_loop() {
    let space = SearchSpace::default();
    let configs = space.all_configs();
    let mut sampler = ThompsonSampler::new(configs.len());
    let mut rng = arkavo_llm::autoresearch::Rng::new(42);

    // Run 10 rounds of Thompson Sampling with simulated results
    let responses = [
        "The capital of France is Paris.",
        "150 kilometers. 60 × 2.5 = 150.",
        "def is_palindrome(s): return s == s[::-1]",
        "Ocean waves crash\nSalt and foam on ancient stones\nTide pulls back again",
        "Exercise improves cardiovascular health, boosts mood, and increases energy levels.",
    ];

    let mut best_wq = 0.0f64;
    let mut best_arm = 0;

    for _round in 0..10 {
        let arm = sampler.select(&mut rng);
        let config = &configs[arm];

        let mut round_wq = 0.0;
        for (i, response) in responses.iter().enumerate() {
            let result = simulate_experiment(&space, config, 42 + i as u32, response);
            round_wq += result.weighted_quality;
        }
        let avg_wq = round_wq / responses.len() as f64;

        // Normalize to [0, 1] for Beta update
        let normalized = (avg_wq / 0.2).clamp(0.0, 1.0);
        sampler.update(arm, normalized);

        if avg_wq > best_wq {
            best_wq = avg_wq;
            best_arm = arm;
        }
    }

    // After 10 rounds, the sampler should have updated priors
    let best_prior = sampler.prior(best_arm);
    assert!(
        best_prior.observations() > 0.0,
        "Best arm should have observations"
    );
    assert!(best_wq > 0.0, "Should have found a positive quality config");
}

#[test]
fn test_gossip_announcement_roundtrip() {
    let space = SearchSpace::default();
    let config = ExperimentConfig {
        enable_thinking: false,
        temperature_idx: 1,
        max_tokens_idx: 0,
        top_p_idx: 2,
        kernel_variant: 0,
    };

    let result = simulate_experiment(&space, &config, 42, "The capital of France is Paris.");

    let prior = BetaPrior {
        alpha: 5.0,
        beta: 2.0,
    };
    let announcement = result_to_announcement(&result, &prior, "node-1", HardwareTier::Desktop);

    // Verify announcement fields
    assert!(!announcement.config_json.is_empty());
    assert!(announcement.weighted_quality > 0.0);
    assert!((announcement.prior_alpha - 5.0).abs() < f64::EPSILON);
    assert!((announcement.prior_beta - 2.0).abs() < f64::EPSILON);

    // Deserialize config from announcement
    let decoded: ExperimentConfig = serde_json::from_str(&announcement.config_json).unwrap();
    assert_eq!(decoded, config);
}

#[test]
fn test_cross_node_thompson_merge() {
    let space = SearchSpace::default();
    let configs = space.all_configs();

    // Node A: explores arm 5, finds it good
    let mut node_a = ThompsonSampler::new(configs.len());
    for _ in 0..20 {
        node_a.update(5, 0.85);
    }

    // Node B: hasn't explored arm 5
    let mut node_b = ThompsonSampler::new(configs.len());
    for _ in 0..20 {
        node_b.update(10, 0.3); // explored arm 10, found it bad
    }

    let arm5_before = node_b.prior(5).mean();

    // Simulate gossip: Node A broadcasts, Node B merges
    node_b.merge_remote(&node_a.priors);

    let arm5_after = node_b.prior(5).mean();

    // Node B should now know arm 5 is promising
    assert!(
        arm5_after > arm5_before,
        "After merge, arm 5 should be more promising: before={arm5_before}, after={arm5_after}"
    );
    assert!(
        arm5_after > 0.7,
        "Arm 5 should have high posterior after merge: {arm5_after}"
    );
}

#[test]
fn test_hardware_tier_normalization() {
    let space = SearchSpace::default();
    let config = space.all_configs()[0].clone();

    // Same experiment on different hardware
    let result = simulate_experiment(&space, &config, 42, "Good answer here.");

    let edge_ann =
        result_to_announcement(&result, &BetaPrior::new(), "edge-node", HardwareTier::Edge);
    let server_ann = result_to_announcement(
        &result,
        &BetaPrior::new(),
        "server-node",
        HardwareTier::Server,
    );

    // Edge results should be weighted higher (more surprising to get good results)
    let edge_weight = edge_ann.hardware_tier.normalization_weight();
    let server_weight = server_ann.hardware_tier.normalization_weight();
    assert!(
        edge_weight > server_weight,
        "Edge results should weigh more: edge={edge_weight}, server={server_weight}"
    );

    // Normalized quality = weighted_quality × hardware_weight
    let edge_normalized = edge_ann.weighted_quality * edge_weight;
    let server_normalized = server_ann.weighted_quality * server_weight;
    assert!(
        edge_normalized > server_normalized,
        "Edge-normalized > server-normalized"
    );
}

#[test]
fn test_experiment_vote_with_reproduction() {
    let vote =
        ExperimentVote::new(uuid::Uuid::new_v4(), "node-2".into(), true).with_reproduction(0.21);

    assert!(vote.accept);
    assert!((vote.reproduced_quality.unwrap() - 0.21).abs() < f64::EPSILON);
    assert!(!vote.content_to_sign().is_empty());
}

#[test]
fn test_thompson_convergence_across_nodes() {
    let space = SearchSpace::default();
    let configs = space.all_configs();

    // Simulate 3 nodes each running 10 rounds independently
    let mut nodes: Vec<ThompsonSampler> = (0..3)
        .map(|_| ThompsonSampler::new(configs.len()))
        .collect();

    let mut rngs: Vec<arkavo_llm::autoresearch::Rng> = (0..3)
        .map(|i| arkavo_llm::autoresearch::Rng::new(42 + i * 1000))
        .collect();

    // Each node explores independently — force some arm 5 exposure
    for node_idx in 0..3 {
        // Ensure each node tries arm 5 at least once with good result
        nodes[node_idx].update(5, 0.85);
        // Then explore randomly
        for _ in 0..15 {
            let arm = nodes[node_idx].select(&mut rngs[node_idx]);
            let quality = if arm == 5 {
                0.85
            } else {
                ((arm % 3) as f64).mul_add(0.1, 0.3)
            };
            nodes[node_idx].update(arm, quality);
        }
    }

    // Now simulate gossip: each node merges all others' priors
    let snapshots: Vec<HashMap<usize, BetaPrior>> =
        nodes.iter().map(|n| n.priors.clone()).collect();

    for (node_idx, node) in nodes.iter_mut().enumerate() {
        for (snap_idx, snapshot) in snapshots.iter().enumerate() {
            if snap_idx != node_idx {
                node.merge_remote(snapshot);
            }
        }
    }

    // After gossip, all nodes should agree arm 5 is best
    for (i, node) in nodes.iter().enumerate() {
        let _best = node.best_arm();
        let arm5_mean = node.prior(5).mean();
        assert!(
            arm5_mean > 0.6,
            "Node {i}: arm 5 should have high posterior after gossip merge: {arm5_mean}"
        );
    }
}

#[test]
fn test_search_space_customization() {
    let space = SearchSpace {
        temperatures: vec![0.3, 0.7],
        max_tokens: vec![64],
        top_p_values: vec![0.9],
        kernel_variants: 2,
        ..SearchSpace::default()
    };

    // 2 thinking × 2 temps × 1 max_tok × 1 top_p × 2 kernel = 8
    assert_eq!(space.config_count(), 8);
    assert_eq!(space.all_configs().len(), 8);

    // Verify kernel variants are enumerated
    let kv_values: Vec<u32> = space
        .all_configs()
        .iter()
        .map(|c| c.kernel_variant)
        .collect();
    assert!(kv_values.contains(&0));
    assert!(kv_values.contains(&1));
}

#[test]
fn test_evaluator_integration_with_results() {
    let space = SearchSpace::default();
    let config = space.all_configs()[0].clone();

    // Good response
    let good = simulate_experiment(&space, &config, 42, "The capital of France is Paris.");
    assert!(good.quality_score > 0.8);
    assert!(good.weighted_quality > 0.0);

    // Empty response
    let empty = simulate_experiment(&space, &config, 42, "");
    assert!(empty.quality_score < f64::EPSILON);
    assert!(empty.weighted_quality < f64::EPSILON);

    // Repetitive response
    let repetitive = simulate_experiment(
        &space,
        &config,
        42,
        "the the the the the the the the the the the the",
    );
    assert!(repetitive.quality_score < good.quality_score);
}
