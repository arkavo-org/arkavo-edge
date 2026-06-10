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
use arkavo_arp::constraints::{ApprovalMechanism, Budget, BudgetExhaustionAction, Velocity};
use arkavo_arp::feedback::{
    DecayStrategy, FeedbackLoops, ImmediateFeedback, PolicyCacheConfig, QualityFailureAction,
    QualityGate, QualityMetric, ShortTermFeedback,
};
use arkavo_arp::model::AdlRef;
use arkavo_arp::proposal::{BlastRadius, ProposalOrigin, ProposalPolicy};
use arkavo_swarmkit::{
    GlobalBudget, ProposalApprovalMechanism, ProposalBlastRadius, ProposalGovernanceSpec,
    ProposalOriginSpec, RoleSpec,
};

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
    derive_arp_for_role_with_governance(role, global, role_count, opts, None)
}

/// Same derivation, plus a per-role `proposal_policy` derived from the kit's
/// `proposal_governance` block — the same kit-config-to-role-ARP pattern as
/// the global budget split. No governance means no proposal policy: the
/// role's queue rejects every ingest.
pub fn derive_arp_for_role_with_governance(
    role: &RoleSpec,
    global: &GlobalBudget,
    role_count: usize,
    opts: DeriveOptions,
    governance: Option<&ProposalGovernanceSpec>,
) -> ArpDocument {
    let proposal_policy = governance.map(|gov| derive_proposal_policy(role, gov));
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
        proposal_policy,
        metadata: None,
    }
}

/// Map the kit-level governance block onto one role's ARP proposal policy.
fn derive_proposal_policy(role: &RoleSpec, gov: &ProposalGovernanceSpec) -> ProposalPolicy {
    // Conservative default when the kit lists no origins: human only.
    let accepted_origins = if gov.accepted_origins.is_empty() {
        vec![ProposalOrigin::Human]
    } else {
        gov.accepted_origins.iter().map(map_origin).collect()
    };
    // Per-role ceiling, defaulting to single_entity. Mesh is rejected at
    // manifest validation; clamp defensively anyway.
    let ceiling = gov
        .role_ceilings
        .iter()
        .find(|c| c.role == role.id)
        .map(|c| map_radius(c.auto_apply_max_blast_radius))
        .unwrap_or(BlastRadius::SingleEntity)
        .min(BlastRadius::Flight);
    ProposalPolicy {
        accepted_origins,
        auto_apply_max_blast_radius: ceiling,
        review: gov.flight_approval.map(map_approval),
        observation_window_sec: gov.observation_window_sec,
    }
}

fn map_origin(o: &ProposalOriginSpec) -> ProposalOrigin {
    match o {
        ProposalOriginSpec::Consolidation => ProposalOrigin::Consolidation,
        ProposalOriginSpec::Critic => ProposalOrigin::Critic,
        ProposalOriginSpec::Historian => ProposalOrigin::Historian,
        ProposalOriginSpec::Human => ProposalOrigin::Human,
    }
}

fn map_radius(r: ProposalBlastRadius) -> BlastRadius {
    match r {
        ProposalBlastRadius::SingleEntity => BlastRadius::SingleEntity,
        ProposalBlastRadius::SingleAgent => BlastRadius::SingleAgent,
        ProposalBlastRadius::Flight => BlastRadius::Flight,
        ProposalBlastRadius::Mesh => BlastRadius::Mesh,
    }
}

fn map_approval(a: ProposalApprovalMechanism) -> ApprovalMechanism {
    match a {
        ProposalApprovalMechanism::CredentialPresentation => {
            ApprovalMechanism::CredentialPresentation
        }
        ProposalApprovalMechanism::SignedCommand => ApprovalMechanism::SignedCommand,
        ProposalApprovalMechanism::InteractiveConfirmation => {
            ApprovalMechanism::InteractiveConfirmation
        }
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
    #[test]
    fn no_governance_derives_no_proposal_policy() {
        let doc = derive_arp_for_role(&role("r1"), &budget(), 1, DeriveOptions::default());
        assert!(doc.proposal_policy.is_none());
    }

    #[test]
    fn governance_derives_per_role_policy() {
        use arkavo_swarmkit::{
            ProposalApprovalMechanism, ProposalBlastRadius, ProposalGovernanceSpec,
            ProposalOriginSpec, RoleProposalCeiling,
        };
        let gov = ProposalGovernanceSpec {
            accepted_origins: vec![ProposalOriginSpec::Consolidation, ProposalOriginSpec::Human],
            role_ceilings: vec![RoleProposalCeiling {
                role: "commander".into(),
                auto_apply_max_blast_radius: ProposalBlastRadius::SingleAgent,
            }],
            flight_approval: Some(ProposalApprovalMechanism::InteractiveConfirmation),
            observation_window_sec: Some(900),
        };

        // Role with an explicit ceiling.
        let doc = derive_arp_for_role_with_governance(
            &role("commander"),
            &budget(),
            2,
            DeriveOptions::default(),
            Some(&gov),
        );
        let policy = doc.proposal_policy.unwrap();
        assert_eq!(policy.auto_apply_max_blast_radius, BlastRadius::SingleAgent);
        assert_eq!(policy.accepted_origins.len(), 2);
        assert_eq!(policy.observation_window_sec, Some(900));
        assert_eq!(
            policy.review,
            Some(ApprovalMechanism::InteractiveConfirmation)
        );

        // Role without an entry derives the single_entity default.
        let doc = derive_arp_for_role_with_governance(
            &role("historian"),
            &budget(),
            2,
            DeriveOptions::default(),
            Some(&gov),
        );
        let policy = doc.proposal_policy.unwrap();
        assert_eq!(
            policy.auto_apply_max_blast_radius,
            BlastRadius::SingleEntity
        );
    }

    #[test]
    fn empty_origins_default_to_human_only() {
        use arkavo_swarmkit::ProposalGovernanceSpec;
        let gov = ProposalGovernanceSpec {
            accepted_origins: vec![],
            role_ceilings: vec![],
            flight_approval: None,
            observation_window_sec: None,
        };
        let doc = derive_arp_for_role_with_governance(
            &role("r1"),
            &budget(),
            1,
            DeriveOptions::default(),
            Some(&gov),
        );
        let policy = doc.proposal_policy.unwrap();
        assert_eq!(policy.accepted_origins, vec![ProposalOrigin::Human]);
    }

    #[test]
    fn derived_document_passes_arp_validation() {
        use arkavo_swarmkit::{
            ProposalBlastRadius, ProposalGovernanceSpec, ProposalOriginSpec, RoleProposalCeiling,
        };
        let gov = ProposalGovernanceSpec {
            accepted_origins: vec![ProposalOriginSpec::Consolidation],
            role_ceilings: vec![RoleProposalCeiling {
                role: "r1".into(),
                auto_apply_max_blast_radius: ProposalBlastRadius::Flight,
            }],
            flight_approval: None,
            observation_window_sec: None,
        };
        let doc = derive_arp_for_role_with_governance(
            &role("r1"),
            &budget(),
            1,
            DeriveOptions::default(),
            Some(&gov),
        );
        assert!(arkavo_arp::validate::validate(&doc).is_ok());
    }
}
