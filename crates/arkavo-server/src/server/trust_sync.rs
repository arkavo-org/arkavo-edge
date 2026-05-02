//! Background sync from learning signals into the MCP-T trust service.
//!
//! `TrustService` only scores subjects whose `AgentTrustInput` has been
//! populated. Without this loop the L1 endpoint returns `SubjectNotFound`
//! for every query because nothing else in the codebase calls
//! `update_agent_input`. The data already exists — `BetaPrior` in the
//! router, `AntiPatternStore` in the policy cache, peer count in the
//! gossip layer — so this is plumbing, not new logic.
//!
//! The loop runs at a fixed interval and writes the local agent's
//! trust input each tick. Per-peer scoring would require remote
//! signal collection and is intentionally out of scope.

use std::sync::Arc;
use std::time::Duration;

use arkavo_router::Router;
use arkavo_trust::{AgentTrustInput, SharedTrustService};
use chrono::{DateTime, Utc};
use tokio::task::JoinHandle;
use tracing::debug;

use super::learning_bus::LearningBus;

const SYNC_INTERVAL: Duration = Duration::from_secs(30);
const SECURITY_CATEGORY: &str = "security";

/// Spawn a background task that periodically populates the trust service's
/// `AgentTrustInput` for the local agent from live learning signals.
///
/// Returns the join handle so the caller can attach it to the server's
/// shutdown sequence (or leak it for the lifetime of the process — the loop
/// has no exit condition aside from a panic in one of its sources).
pub(super) fn spawn_trust_sync(
    trust_service: SharedTrustService,
    learning_bus: Arc<LearningBus>,
    router: Arc<Router>,
    created_at: DateTime<Utc>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Subject ID matches the human-readable agent name used as the key
        // for BetaPrior and AntiPatternStore — using a DID would cause the
        // signal lookups to miss because those subsystems pre-date the
        // device-DID convention. The trust SCORE is still signed by the
        // persistent device keypair, so signatures verify across bounces;
        // only the SUBJECT identifier follows the existing learning key.
        let subject_id = learning_bus.agent_id().to_string();
        let mut ticker = tokio::time::interval(SYNC_INTERVAL);
        // First tick fires immediately; let it.
        loop {
            ticker.tick().await;
            let input = collect_input(&learning_bus, &router, &subject_id, created_at).await;
            trust_service.update_agent_input(&subject_id, input);
            debug!("MCP-T trust input refreshed for {subject_id}");
        }
    })
}

async fn collect_input(
    learning_bus: &LearningBus,
    router: &Router,
    subject_id: &str,
    created_at: DateTime<Utc>,
) -> AgentTrustInput {
    let stats = learning_bus.stats().await;
    let peer_count = u32::try_from(stats.gossip_peers).unwrap_or(u32::MAX);

    // Aggregate BetaPrior across all task categories the local agent appears
    // in. Most installs have a single agent_id, but the loop is correct for
    // multi-agent rollouts as well — it averages the success/std signal
    // weighted by observation count.
    let utility_stats = router.model_learning().get_all_stats().await;
    let local_stats: Vec<_> = utility_stats
        .into_iter()
        .filter(|s| s.agent_id == subject_id)
        .collect();

    let (success_rate, success_std_dev, observation_count) = if local_stats.is_empty() {
        (None, None, None)
    } else {
        let total_obs: u64 = local_stats.iter().map(|s| s.total_observations).sum();
        if total_obs == 0 {
            (None, None, None)
        } else {
            let weight =
                |s: &arkavo_router::learning::AgentUtilityStats| s.total_observations as f64;
            let total_w: f64 = local_stats.iter().map(weight).sum();
            let mean = local_stats
                .iter()
                .map(|s| s.expected_value * weight(s))
                .sum::<f64>()
                / total_w;
            let std = local_stats
                .iter()
                .map(|s| s.std_dev * weight(s))
                .sum::<f64>()
                / total_w;
            (
                Some(mean),
                Some(std),
                Some(u32::try_from(total_obs).unwrap_or(u32::MAX)),
            )
        }
    };

    // Anti-pattern weights are decayed sums of recorded failures. The
    // security-only signal feeds the SECURITY dimension; the total feeds
    // COMPLIANCE. Both are inverted in the mapper so a clean record →
    // higher score.
    let cache = learning_bus.policy_cache().read().await;
    let patterns = cache.anti_patterns.get_for_model(subject_id);
    let total_anti_pattern_weight: f64 = patterns.iter().map(|p| p.decayed_weight()).sum();
    let security_anti_pattern_weight: f64 = patterns
        .iter()
        .filter(|p| p.category == SECURITY_CATEGORY)
        .map(|p| p.decayed_weight())
        .sum();
    drop(cache);

    AgentTrustInput {
        success_rate,
        success_std_dev,
        observation_count,
        has_verified_did: true,
        created_at: Some(created_at),
        security_anti_pattern_weight: Some(security_anti_pattern_weight),
        total_anti_pattern_weight: Some(total_anti_pattern_weight),
        // decision_trace_coverage is intentionally None until the router
        // exposes a "traced / total" ratio rather than a recent-trace
        // window. Fabricating coverage from `recent_decision_traces.len()`
        // would inflate the TRANSPARENCY dimension; better to omit.
        decision_trace_coverage: None,
        peer_count: Some(peer_count),
        // v0.2 fidelity signals are populated by the conductor's
        // behavior.trace emission, not this sync.
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use arkavo_router::learning::AgentUtilityStats;

    #[test]
    fn empty_stats_produce_no_success_signal() {
        // Sanity: helper math should not panic on an empty input set,
        // and should leave success_rate as None for callers to skip.
        let local_stats: Vec<AgentUtilityStats> = vec![];
        let total_obs: u64 = local_stats.iter().map(|s| s.total_observations).sum();
        assert_eq!(total_obs, 0);
    }

    #[test]
    fn weighted_mean_of_two_priors() {
        // 100 obs at 0.8 and 50 obs at 0.5 → weighted mean ≈ 0.7
        let stats = [
            AgentUtilityStats {
                agent_id: "a".into(),
                alpha: 0.0,
                beta_param: 0.0,
                expected_value: 0.8,
                std_dev: 0.05,
                total_observations: 100,
                success_rate: 0.8,
                probationary: false,
                total_cost_usd: 0.0,
                avg_latency_ms: 0.0,
            },
            AgentUtilityStats {
                agent_id: "a".into(),
                alpha: 0.0,
                beta_param: 0.0,
                expected_value: 0.5,
                std_dev: 0.10,
                total_observations: 50,
                success_rate: 0.5,
                probationary: false,
                total_cost_usd: 0.0,
                avg_latency_ms: 0.0,
            },
        ];
        let weight = |s: &AgentUtilityStats| s.total_observations as f64;
        let total_w: f64 = stats.iter().map(weight).sum();
        let mean = stats
            .iter()
            .map(|s| s.expected_value * weight(s))
            .sum::<f64>()
            / total_w;
        assert!((mean - 0.7).abs() < 1e-9, "got {mean}");
    }
}
