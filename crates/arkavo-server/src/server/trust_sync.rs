//! Background sync from learning signals into the MCP-T trust service.
//!
//! `TrustService` only scores subjects whose `AgentTrustInput` has been
//! populated. Without this loop the L1 endpoint returns `SubjectNotFound`
//! for every query.
//!
//! Trust **subjects** are agents — the local agent plus each peer the
//! local gossip layer knows about. They are NOT models. Router model
//! stats (`LearningModule::get_all_stats`) record the local agent's
//! Thompson-Sampling posteriors over MODEL VARIANTS it has used; the
//! `agent_id` field in those stats is the model name, which made an
//! earlier version of this loop emit one trust subject per model. That
//! was wrong: models are an implementation detail of the local agent's
//! performance, not external entities to score.
//!
//! Source map per dimension on each subject:
//! - **self**: `success_rate`/`std_dev` aggregated across model_learning
//!   stats (the local agent's effective track record across the models
//!   it has selected); `created_at` from the persistent device-keypair
//!   file mtime; `peer_count` from gossip; anti-pattern weights from
//!   the policy cache for the local agent's name.
//! - **peer**: `verification: true` (we know they exist on the gossip
//!   layer); other signals empty until per-peer attestation lands. The
//!   panel renders an entry so operators can see the peer set even when
//!   no scoring evidence exists yet.

use std::sync::Arc;
use std::time::Duration;

use arkavo_router::Router;
use arkavo_router::learning::AgentUtilityStats;
use arkavo_trust::{AgentTrustInput, SharedTrustService};
use chrono::{DateTime, Utc};
use tokio::task::JoinHandle;
use tracing::debug;

use super::anti_pattern::AntiPattern;
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
        // Subject ID for the LOCAL agent matches the human-readable agent
        // name used as the key for BetaPrior and AntiPatternStore — using a
        // DID would cause those subsystem lookups to miss. The trust SCORE
        // is still signed by the persistent device keypair, so signatures
        // verify across bounces; only the SUBJECT identifier follows the
        // existing learning key. Peer subjects use the same agent_id space.
        let local_subject_id = learning_bus.agent_id().to_string();
        let mut ticker = tokio::time::interval(SYNC_INTERVAL);
        loop {
            ticker.tick().await;
            sync_once(
                &trust_service,
                &learning_bus,
                &router,
                &local_subject_id,
                created_at,
            )
            .await;
        }
    })
}

async fn sync_once(
    trust_service: &SharedTrustService,
    learning_bus: &LearningBus,
    router: &Router,
    local_subject_id: &str,
    created_at: DateTime<Utc>,
) {
    let stats = learning_bus.stats().await;
    let local_peer_count = u32::try_from(stats.gossip_peers).unwrap_or(u32::MAX);

    // SELF: aggregate every model_learning entry into one weighted-mean
    // success signal. These records track the local agent's per-model
    // Thompson-Sampling posteriors; their union is the agent's effective
    // track record across the models it has chosen.
    let local_model_stats = router.model_learning().get_all_stats().await;
    let cache = learning_bus.policy_cache().read().await;
    let local_patterns = cache.anti_patterns.get_for_model(local_subject_id);
    let local_input = build_input(
        &local_model_stats,
        &local_patterns,
        BuildContext {
            created_at: Some(created_at),
            peer_count: Some(local_peer_count),
        },
    );
    trust_service.update_agent_input(local_subject_id, local_input);

    // PEERS: enumerate from the gossip layer — these are real other
    // agents the local node sees on the mesh. We have no scoring
    // evidence about them yet (per-peer attestation/cross-agent
    // outcomes are a follow-up), so the AgentTrustInput is empty
    // beyond the verification flag. The panel still renders an entry
    // per peer so operators can see the peer set.
    let peer_ids = learning_bus.peer_ids().await;
    let peer_count = peer_ids.len();
    for peer_id in &peer_ids {
        let patterns = cache.anti_patterns.get_for_model(peer_id);
        let input = build_input(
            &[],
            &patterns,
            BuildContext {
                created_at: None,
                peer_count: None,
            },
        );
        trust_service.update_agent_input(peer_id, input);
    }
    drop(cache);

    debug!("MCP-T trust input refreshed: local={local_subject_id} peers={peer_count}");
}

/// Per-subject context that comes from outside the router/anti-pattern data.
/// Fields are `Some` only when meaningful — e.g. `created_at` is set for the
/// local agent (we know when *we* started) but not for peers (we don't know
/// when they came online).
struct BuildContext {
    created_at: Option<DateTime<Utc>>,
    peer_count: Option<u32>,
}

fn build_input(
    subject_stats: &[AgentUtilityStats],
    patterns: &[&AntiPattern],
    ctx: BuildContext,
) -> AgentTrustInput {
    let (success_rate, success_std_dev, observation_count) = if subject_stats.is_empty() {
        (None, None, None)
    } else {
        let total_obs: u64 = subject_stats.iter().map(|s| s.total_observations).sum();
        if total_obs == 0 {
            (None, None, None)
        } else {
            let weight = |s: &AgentUtilityStats| s.total_observations as f64;
            let total_w: f64 = subject_stats.iter().map(weight).sum();
            let mean = subject_stats
                .iter()
                .map(|s| s.expected_value * weight(s))
                .sum::<f64>()
                / total_w;
            let std = subject_stats
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
    let total_anti_pattern_weight: f64 = patterns.iter().map(|p| p.decayed_weight()).sum();
    let security_anti_pattern_weight: f64 = patterns
        .iter()
        .filter(|p| p.category == SECURITY_CATEGORY)
        .map(|p| p.decayed_weight())
        .sum();

    AgentTrustInput {
        success_rate,
        success_std_dev,
        observation_count,
        // Peers are recognized via the same identity layer that gossip uses,
        // so within the local provider's view they're "verified" in the sense
        // MCP-T means: we trust the identifier resolves to a stable entity.
        // Stronger identity attestation is a follow-up.
        has_verified_did: true,
        created_at: ctx.created_at,
        security_anti_pattern_weight: Some(security_anti_pattern_weight),
        total_anti_pattern_weight: Some(total_anti_pattern_weight),
        // decision_trace_coverage is intentionally None until the router
        // exposes a "traced / total" ratio rather than a recent-trace
        // window. Fabricating coverage from `recent_decision_traces.len()`
        // would inflate the TRANSPARENCY dimension; better to omit.
        decision_trace_coverage: None,
        peer_count: ctx.peer_count,
        // v0.2 fidelity signals are populated by the conductor's
        // behavior.trace emission, not this sync.
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(
        agent_id: &str,
        expected_value: f64,
        std_dev: f64,
        observations: u64,
    ) -> AgentUtilityStats {
        AgentUtilityStats {
            agent_id: agent_id.into(),
            alpha: 0.0,
            beta_param: 0.0,
            expected_value,
            std_dev,
            total_observations: observations,
            success_rate: expected_value,
            probationary: false,
            total_cost_usd: 0.0,
            avg_latency_ms: 0.0,
        }
    }

    #[test]
    fn build_input_empty_stats_produce_no_success_signal() {
        let input = build_input(
            &[],
            &[],
            BuildContext {
                created_at: None,
                peer_count: None,
            },
        );
        assert_eq!(input.success_rate, None);
        assert_eq!(input.observation_count, None);
        assert!(input.has_verified_did);
    }

    #[test]
    fn build_input_weighted_mean_of_two_priors() {
        // 100 obs at 0.8 and 50 obs at 0.5 → weighted mean ≈ 0.7
        let stats = [stat("a", 0.8, 0.05, 100), stat("a", 0.5, 0.10, 50)];
        let input = build_input(
            &stats,
            &[],
            BuildContext {
                created_at: None,
                peer_count: None,
            },
        );
        let mean = input.success_rate.expect("success_rate populated");
        assert!((mean - 0.7).abs() < 1e-9, "got {mean}");
        assert_eq!(input.observation_count, Some(150));
    }

    #[test]
    fn build_input_peer_omits_local_only_signals() {
        // Peers must not get a tenure score (we don't know when they
        // started) or a community score (peer_count is the local agent's
        // view, not the peer's). The mapper omits dimensions whose input
        // is None, so leaving these fields empty is the correct signal.
        let stats = [stat("peer-x", 0.9, 0.05, 200)];
        let input = build_input(
            &stats,
            &[],
            BuildContext {
                created_at: None,
                peer_count: None,
            },
        );
        assert_eq!(input.created_at, None);
        assert_eq!(input.peer_count, None);
        // But the actual evidence we DO have (BetaPrior) is preserved.
        assert!(input.success_rate.is_some());
    }

    #[test]
    fn build_input_local_carries_tenure_and_community() {
        let stats = [stat("self", 0.85, 0.04, 300)];
        let now = Utc::now();
        let input = build_input(
            &stats,
            &[],
            BuildContext {
                created_at: Some(now),
                peer_count: Some(7),
            },
        );
        assert_eq!(input.created_at, Some(now));
        assert_eq!(input.peer_count, Some(7));
    }
}
