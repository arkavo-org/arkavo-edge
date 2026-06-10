//! Derive a default ARP document from a SwarmKit role's `agent_provisioning`
//! block when the launcher does not supply an explicit ARP override.
//!
//! The derivation is deliberately conservative: pick safe defaults that match
//! the existing arkavo-arp-runtime test fixtures (Thompson Sampling adaptation,
//! 0.7 quality gate, 1-hour cache TTL with exponential decay), and translate
//! the SwarmKit-side per-flight budget into ARP's per-task USD budget by
//! splitting the global cost cap across all roles.
//!
//! Producers that need finer control supply a hand-authored ARP via
//! `LaunchOptions::arp_overrides`.

use arkavo_arp::ArpDocument;
use arkavo_arp::adaptation::{Adaptation, AdaptationMethod};
use arkavo_arp::constraints::{Budget, BudgetExhaustionAction, Velocity};
use arkavo_arp::feedback::{
    DecayStrategy, FeedbackLoops, ImmediateFeedback, PolicyCacheConfig, QualityFailureAction,
    QualityGate, QualityMetric, ShortTermFeedback,
};
use arkavo_arp::model::AdlRef;
use arkavo_swarmkit::{GlobalBudget, RoleSpec};

/// Tunables for the default-ARP derivation.
///
/// **Rationale for the defaults below.** The SwarmKit specification does not
/// prescribe specific ARP runtime values for derived flights — it only
/// requires that a derivation exist (spec §1.2 / §5 hand-off language). The
/// numbers chosen here are ours, intended to track the existing
/// arkavo-arp-runtime conventions so a flight launched from a manifest looks
/// indistinguishable to the panel from a manually-loaded ARP document:
///
/// * `quality_threshold = 0.7` — matches the spec §4.6 baseline rubric's
///   accuracy/completeness thresholds and the existing PR #572 ARP showcase.
/// * `cache_ttl_sec = 3600` — matches the arkavo-arp-runtime test fixture and
///   keeps short-lived flights (90-day max horizon per §10.1) well inside the
///   bound.
/// * `cache_half_life_sec = 86400` — mirrors the showcase's exponential-decay
///   tuning. Beta priors compete with constitutional policy on a daily scale.
/// * `velocity_window_minutes = 5.0` — divides `task_ceiling_usd` over a
///   5-minute spending window. Matches the budget velocity defaults the
///   conductor enforces today; chosen because most agent tasks are bounded
///   by single-digit minutes of LLM inference.
///
/// All four are tunable. Producers that need different defaults supply a
/// hand-authored ARP via `LaunchOptions::arp_overrides`, which bypasses
/// derivation entirely.
#[derive(Debug, Clone, Copy)]
pub struct DeriveOptions {
    /// Quality gate threshold below which an outcome is treated as a failure
    /// when updating priors.
    pub quality_threshold: f64,
    /// Cache TTL applied to every PolicyCache entry.
    pub cache_ttl_sec: u64,
    /// Decay half-life for cache entries.
    pub cache_half_life_sec: u64,
    /// Per-minute spend velocity ceiling derived as
    /// `task_ceiling_usd / velocity_window_minutes`.
    pub velocity_window_minutes: f64,
}

impl Default for DeriveOptions {
    fn default() -> Self {
        Self {
            quality_threshold: 0.7,
            cache_ttl_sec: 3600,
            cache_half_life_sec: 86_400,
            velocity_window_minutes: 5.0,
        }
    }
}

/// Build a minimal ARP document for one role.
///
/// `role_count` is used to split the SwarmKit `global_budget.max_cost_usd`
/// across roles so each role gets its own ARP `task_ceiling_usd`.
pub fn derive_arp_for_role(
    role: &RoleSpec,
    global: &GlobalBudget,
    role_count: usize,
    opts: DeriveOptions,
) -> ArpDocument {
    let role_count = role_count.max(1) as f64;
    let task_ceiling = (global.max_cost_usd / role_count).max(f64::EPSILON);
    let velocity_per_min = (task_ceiling / opts.velocity_window_minutes).max(f64::EPSILON);

    ArpDocument {
        arp_spec: "0.1.0".into(),
        adl_ref: AdlRef {
            uri: Some(format!("urn:swarmkit:role:{}", role.id)),
            document_hash: None,
        },
        integrity: None,
        adaptation: Adaptation {
            method: AdaptationMethod::ThompsonSampling,
            parameters: None,
            cold_start: None,
            prior_management: None,
            signal_separation: None,
        },
        feedback_loops: FeedbackLoops {
            immediate: ImmediateFeedback {
                quality_gate: QualityGate {
                    threshold_default: opts.quality_threshold,
                    metric: QualityMetric::Composite,
                    on_failure: QualityFailureAction::UpdatePriorAndLog,
                    threshold_overrides: None,
                },
            },
            short_term: ShortTermFeedback {
                policy_cache: PolicyCacheConfig {
                    default_ttl_sec: opts.cache_ttl_sec,
                    decay_strategy: DecayStrategy::Exponential,
                    decay_half_life_sec: Some(opts.cache_half_life_sec),
                    human_source_exempt_from_decay: None,
                    incident_source_quarantine_sec: None,
                },
            },
            gossip: None,
            consolidation: None,
            resilience: None,
        },
        precedence: None,
        cognitive: None,
        execution: None,
        data_sovereignty: None,
        network: None,
        budget: Budget {
            task_ceiling_usd: task_ceiling,
            on_exhaustion: BudgetExhaustionAction::HaltAndReport,
            degradation_chain: None,
            alert_threshold_pct: None,
            velocity: Velocity {
                max_spend_per_minute_usd: velocity_per_min,
                max_tool_calls_per_minute: None,
                max_tokens_per_minute: None,
            },
            per_layer: None,
            rate_limiting: None,
            accounting: None,
        },
        escalation: None,
        quarantine: None,
        hitl: None,
        session: None,
        state_storage: None,
        observability: None,
        proposal_policy: None,
        metadata: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_swarmkit::{AgentProvisioning, RoleSpec};
    use arkavo_test_macros::spec;

    fn role(id: &str) -> RoleSpec {
        RoleSpec {
            id: id.into(),
            role_type: "specialist".into(),
            description: None,
            agent_provisioning: AgentProvisioning::default(),
            skills: vec![],
            mcp_tools: vec![],
            tdf_attribute_release_policy: None,
            handoffs: vec![],
            context_scope: None,
        }
    }

    fn budget() -> GlobalBudget {
        GlobalBudget {
            max_wallclock_seconds: 300,
            max_total_tokens: 60_000,
            max_cost_usd: 0.30,
        }
    }

    #[spec("SK-013")]
    #[test]
    fn derives_thompson_sampling_default() {
        let doc = derive_arp_for_role(&role("r1"), &budget(), 3, DeriveOptions::default());
        assert_eq!(doc.adaptation.method, AdaptationMethod::ThompsonSampling);
        assert_eq!(
            doc.feedback_loops.immediate.quality_gate.threshold_default,
            0.7
        );
    }

    #[spec("SK-013")]
    #[test]
    fn splits_global_cost_across_roles() {
        let doc = derive_arp_for_role(&role("r1"), &budget(), 3, DeriveOptions::default());
        assert!((doc.budget.task_ceiling_usd - 0.10).abs() < 1e-9);
    }

    #[spec("SK-013")]
    #[test]
    fn velocity_derived_from_ceiling_and_window() {
        let opts = DeriveOptions {
            velocity_window_minutes: 5.0,
            ..DeriveOptions::default()
        };
        let doc = derive_arp_for_role(&role("r1"), &budget(), 3, opts);
        // 0.10 USD / 5 min = 0.02 USD/min
        assert!((doc.budget.velocity.max_spend_per_minute_usd - 0.02).abs() < 1e-9);
    }

    #[spec("SK-013")]
    #[test]
    fn adl_ref_carries_role_id() {
        let doc = derive_arp_for_role(&role("planner-1"), &budget(), 1, DeriveOptions::default());
        assert_eq!(
            doc.adl_ref.uri.as_deref(),
            Some("urn:swarmkit:role:planner-1")
        );
    }

    #[test]
    fn zero_role_count_does_not_divide_by_zero() {
        let doc = derive_arp_for_role(&role("r1"), &budget(), 0, DeriveOptions::default());
        assert!(doc.budget.task_ceiling_usd > 0.0);
    }
}
