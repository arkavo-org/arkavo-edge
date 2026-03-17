//! Autoresearch execution mode for the Conductor
//!
//! Detects `[autoresearch]` marker in task content and runs Thompson Sampling
//! parameter sweeps instead of the normal tool loop. Each experiment evaluates
//! a config (enable_thinking × max_tokens) against a set of prompts, measures
//! wall-clock timing, and feeds quality back into the sampler.

use super::learning_bus::LearningBus;
use arkavo_hrm::{
    Conductor, burst::BurstResult, schemas::SubTaskBudget, schemas::TaskBudget,
    store::InMemoryTaskStore,
};
use arkavo_llm::autoresearch::{
    ExperimentConfig, ExperimentResult, Rng, SearchSpace, ThompsonSampler, evaluate_quality,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Check if a task should be routed to autoresearch mode
pub(super) fn is_autoresearch_task(task_content: &str) -> bool {
    task_content.contains("[autoresearch]")
}

/// Extract the test prompt from autoresearch task content
///
/// Format: `[autoresearch] <prompt to evaluate>`
/// If no prompt follows the marker, uses a default evaluation prompt.
fn extract_prompts(task_content: &str) -> Vec<String> {
    let after_marker = task_content
        .split_once("[autoresearch]")
        .map(|x| x.1)
        .unwrap_or("")
        .trim();

    if after_marker.is_empty() {
        // Default evaluation prompts covering different capabilities
        vec![
            "What is the capital of France?".into(),
            "Write a function that checks if a number is prime.".into(),
            "Explain why the sky is blue in two sentences.".into(),
            "List three benefits of regular exercise.".into(),
        ]
    } else {
        vec![after_marker.to_string()]
    }
}

/// Run an autoresearch parameter sweep via the Conductor
///
/// Creates a task with autoresearch budget, runs Thompson Sampling rounds,
/// and returns a summary of the best configuration found.
#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_autoresearch_sweep(
    conductor: &Arc<Conductor<InMemoryTaskStore>>,
    router: &Arc<arkavo_router::Router>,
    task_content: &str,
    learning_bus: Option<&Arc<LearningBus>>,
    compute_budget: Option<&arkavo_budget::SharedComputeBudget>,
    model_hint: Option<&arkavo_router::ModelChoice>,
) -> Result<String, String> {
    let prompts = extract_prompts(task_content);
    let space = SearchSpace::default();
    let configs = space.all_configs();
    let mut sampler = ThompsonSampler::new(configs.len());
    let mut rng = Rng::from_time();

    // Create HRM task with autoresearch budget
    let budget = TaskBudget::default();
    let hrm_task = conductor
        .create_task("[autoresearch] parameter sweep".into(), budget)
        .await
        .map_err(|e| format!("Failed to create autoresearch task: {e}"))?;

    let subtask_budget = SubTaskBudget::autoresearch();
    let sweep_deadline = Instant::now() + subtask_budget.max_wall_time;
    let max_rounds = space.max_rounds.min(subtask_budget.max_steps) as usize;

    info!(
        task_id = %hrm_task.id,
        configs = configs.len(),
        max_rounds,
        budget_secs = subtask_budget.max_wall_time.as_secs(),
        "Starting autoresearch sweep"
    );

    let mut best_wq = 0.0f64;
    let mut best_arm: Option<usize> = None;
    let mut total_experiments = 0u32;
    let mut results_log: Vec<(usize, f64, Duration)> = Vec::new();

    for round in 0..max_rounds {
        if Instant::now() >= sweep_deadline {
            info!(round, "Autoresearch budget exhausted");
            break;
        }

        // Check compute budget if available
        if let Some(cb) = compute_budget {
            let budget_guard = cb.read().await;
            if budget_guard.remaining_inferences == 0 {
                warn!(round, "Compute budget exhausted during autoresearch");
                break;
            }
        }

        let arm = sampler.select(&mut rng);
        let config = &configs[arm];
        let Some((_temperature, _max_tokens, _top_p)) = space.resolve(config) else {
            warn!(arm, "Config index out of bounds, skipping");
            continue;
        };

        // Build messages with thinking directive
        let thinking_directive = if config.enable_thinking {
            "Think through this problem step-by-step before providing your answer."
        } else {
            "Answer directly and concisely."
        };

        let mut round_wq = 0.0;
        let round_start = Instant::now();

        for prompt in &prompts {
            let experiment_start = Instant::now();

            let messages = vec![
                arkavo_llm::Message::system(thinking_directive.to_string()),
                arkavo_llm::Message::user(prompt.clone()),
            ];

            // Use router's quality gate for inference (respects model hints, semaphore, etc.)
            let response = match tokio::time::timeout(
                space.experiment_timeout,
                router.route_with_tools_hinted(
                    prompt, messages, None, // no tools for experiments
                    model_hint,
                ),
            )
            .await
            {
                Ok(Ok(resp)) => resp.content,
                Ok(Err(e)) => {
                    warn!(arm, prompt = prompt.as_str(), "Experiment failed: {e}");
                    String::new()
                }
                Err(_) => {
                    warn!(arm, "Experiment timed out");
                    String::new()
                }
            };

            let wall_time = experiment_start.elapsed();
            let quality_score = evaluate_quality(&response);
            // Whitespace split approximates token count (~1.3x underestimate for English).
            // Acceptable here since it's consistent across arms within a sweep.
            let tokens_generated = response.split_whitespace().count() as u32;
            let quality_per_token = if tokens_generated > 0 {
                quality_score / tokens_generated as f64
            } else {
                0.0
            };
            let tok_per_sec = if wall_time.as_secs_f64() > 0.0 && tokens_generated > 0 {
                tokens_generated as f64 / wall_time.as_secs_f64()
            } else {
                0.0
            };

            let weighted = ExperimentResult::compute_weighted(quality_per_token, tok_per_sec);
            round_wq += weighted;
            total_experiments += 1;

            // Consume inference from compute budget
            if let Some(cb) = compute_budget {
                let mut budget_guard = cb.write().await;
                budget_guard.consume_inference(
                    u64::from(tokens_generated),
                    0.0, // local model, no cost
                );
            }
        }

        let avg_wq = round_wq / prompts.len() as f64;
        let round_wall = round_start.elapsed();

        // Normalize to [0, 1] for Beta update
        // 0.2 is the empirical ceiling for weighted quality across sweep experiments:
        // quality_per_token (~0.01-0.03) × ln(1 + tok/s ~175) ≈ 0.05-0.15, rarely exceeds 0.2.
        let normalized = (avg_wq / 0.2).clamp(0.0, 1.0);
        sampler.update(arm, normalized);

        results_log.push((arm, avg_wq, round_wall));

        if avg_wq > best_wq {
            best_wq = avg_wq;
            best_arm = Some(arm);
        }
    }

    // Record result in Conductor
    let result_summary = format_sweep_summary(
        &space,
        &configs,
        &sampler,
        best_arm,
        best_wq,
        total_experiments,
        &results_log,
    );

    let subtask = conductor
        .add_subtask(hrm_task.id, "[autoresearch] sweep".into(), vec![])
        .await
        .map_err(|e| format!("Failed to add subtask: {e}"))?;

    let contract = conductor
        .create_contract(&subtask)
        .await
        .map_err(|e| format!("Failed to create contract: {e}"))?;

    let burst_result = BurstResult::success(
        contract.id,
        serde_json::json!({
            "type": "autoresearch_sweep",
            "best_arm": best_arm,
            "best_weighted_quality": best_wq,
            "total_experiments": total_experiments,
        }),
    );
    conductor
        .record_result(hrm_task.id, subtask.id, burst_result)
        .await
        .map_err(|e| format!("Failed to record result: {e}"))?;

    // Determine model name for attribution
    let model_name = model_hint
        .map(|m| m.name().to_string())
        .or_else(|| router.last_routed_model())
        .unwrap_or_default();

    // Update local optimal config store with sweep results
    if let Some(arm) = best_arm {
        let config = &configs[arm];
        if let Some((temp, _max_tok, top_p)) = space.resolve(config) {
            let thinking = if config.enable_thinking {
                arkavo_llm::ThinkingMode::On
            } else {
                arkavo_llm::ThinkingMode::Off
            };
            let posterior_mean = sampler.prior(arm).mean();
            if let Some(model) = arkavo_router::ModelChoice::from_name(&model_name) {
                router.optimal_configs.update_from_sweep(
                    &model,
                    temp,
                    top_p,
                    thinking,
                    posterior_mean,
                );
            }
        }
    }

    // Announce best result via gossip if learning bus is available
    if let (Some(bus), Some(arm)) = (learning_bus, best_arm) {
        announce_experiment_result(bus, &configs[arm], &sampler, arm, best_wq, &model_name).await;
    }

    info!(
        task_id = %hrm_task.id,
        total_experiments,
        best_wq,
        "Autoresearch sweep complete"
    );

    Ok(result_summary)
}

/// Announce the best experiment result via gossip for cross-node learning
async fn announce_experiment_result(
    bus: &Arc<LearningBus>,
    config: &ExperimentConfig,
    sampler: &ThompsonSampler,
    arm: usize,
    best_wq: f64,
    model_name: &str,
) {
    use arkavo_gossip::experiment_message::{
        ExperimentAnnouncement, ExperimentMetrics, HardwareTier,
    };

    let config_json = match serde_json::to_string(config) {
        Ok(j) => j,
        Err(_) => return,
    };

    let prior = sampler.prior(arm);
    let announcement = ExperimentAnnouncement::new(
        uuid::Uuid::new_v4(),
        [0u8; 32],
        bus.agent_id().to_string(),
        HardwareTier::Desktop, // detect at runtime in future
        config_json,
        model_name.to_string(),
        ExperimentMetrics {
            weighted_quality: best_wq,
            baseline_tok_per_sec: 0.0,
            kept: true,
        },
    )
    .with_swarm(bus.swarm_id().to_string())
    .with_prior(prior.alpha, prior.beta);

    let gossip = bus.gossip().read().await;
    let peers = gossip.select_propagation_peers(None).await;
    drop(gossip);

    let peer_count = peers.len();
    for peer_id in peers {
        let _ = bus.gossip_out_tx().send((
            peer_id,
            arkavo_gossip::GossipMessage::ExperimentAnnounce(announcement.clone()),
        ));
    }

    if peer_count > 0 {
        info!(
            "Announced autoresearch result to {} peers (wq={best_wq:.4})",
            peer_count
        );
    }
}

/// Format a human-readable summary of the sweep results
fn format_sweep_summary(
    space: &SearchSpace,
    configs: &[ExperimentConfig],
    sampler: &ThompsonSampler,
    best_arm: Option<usize>,
    best_wq: f64,
    total_experiments: u32,
    results_log: &[(usize, f64, Duration)],
) -> String {
    use std::fmt::Write;

    let mut summary = String::from("## Autoresearch Parameter Sweep Results\n\n");

    let _ = write!(
        summary,
        "- **Experiments**: {total_experiments}\n- **Rounds**: {}\n- **Search space**: {} configs\n\n",
        results_log.len(),
        configs.len()
    );

    if let Some(arm) = best_arm {
        let config = &configs[arm];
        let (temp, max_tok, top_p) = space.resolve(config).unwrap_or((0.7, 128, 0.9));
        let prior = sampler.prior(arm);
        let _ = write!(
            summary,
            "### Best Configuration (arm {})\n\n\
             - **Thinking**: {}\n\
             - **Temperature**: {temp}\n\
             - **Max tokens**: {max_tok}\n\
             - **Top-p**: {top_p}\n\
             - **Weighted quality**: {best_wq:.4}\n\
             - **Prior**: Beta({:.2}, {:.2}) mean={:.3}\n\n",
            arm,
            if config.enable_thinking {
                "enabled"
            } else {
                "disabled"
            },
            prior.alpha,
            prior.beta,
            prior.mean()
        );
    }

    // Top 5 arms by posterior mean
    summary.push_str("### Top 5 Arms by Posterior\n\n");
    let mut arms: Vec<(usize, f64)> = (0..configs.len())
        .map(|i| (i, sampler.prior(i).mean()))
        .filter(|(_, mean)| (*mean - 0.5).abs() > f64::EPSILON) // skip unexplored
        .collect();
    arms.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (i, (arm, mean)) in arms.iter().take(5).enumerate() {
        let config = &configs[*arm];
        let (temp, max_tok, _) = space.resolve(config).unwrap_or((0.7, 128, 0.9));
        let _ = writeln!(
            summary,
            "{}. arm={arm}: thinking={}, temp={temp}, max_tok={max_tok}, mean={mean:.3}",
            i + 1,
            config.enable_thinking
        );
    }

    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_autoresearch_task() {
        assert!(is_autoresearch_task("[autoresearch] test"));
        assert!(is_autoresearch_task("prefix [autoresearch] suffix"));
        assert!(!is_autoresearch_task("normal task"));
        assert!(!is_autoresearch_task("[research] not auto"));
    }

    #[test]
    fn test_extract_prompts_default() {
        let prompts = extract_prompts("[autoresearch]");
        assert_eq!(prompts.len(), 4);
        assert!(prompts[0].contains("capital"));
    }

    #[test]
    fn test_extract_prompts_custom() {
        let prompts = extract_prompts("[autoresearch] What is 2+2?");
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0], "What is 2+2?");
    }

    #[test]
    fn test_format_sweep_summary_empty() {
        let space = SearchSpace::default();
        let configs = space.all_configs();
        let sampler = ThompsonSampler::new(configs.len());
        let summary = format_sweep_summary(&space, &configs, &sampler, None, 0.0, 0, &[]);
        assert!(summary.contains("Autoresearch"));
        assert!(summary.contains("Experiments**: 0"));
    }

    #[test]
    fn test_format_sweep_summary_with_results() {
        let space = SearchSpace::default();
        let configs = space.all_configs();
        let mut sampler = ThompsonSampler::new(configs.len());
        sampler.update(0, 0.8);
        sampler.update(1, 0.3);

        let log = vec![
            (0, 0.18, Duration::from_millis(500)),
            (1, 0.12, Duration::from_millis(600)),
        ];
        let summary = format_sweep_summary(&space, &configs, &sampler, Some(0), 0.18, 8, &log);
        assert!(summary.contains("Best Configuration"));
        assert!(summary.contains("Top 5"));
    }

    #[test]
    fn test_subtask_budget_autoresearch() {
        let budget = SubTaskBudget::autoresearch();
        assert_eq!(budget.max_steps, 100);
        assert_eq!(budget.max_wall_time, Duration::from_secs(300));
        assert!(budget.max_cost_usd.abs() < f64::EPSILON);
        assert_eq!(budget.max_tokens, 500_000);
    }
}
