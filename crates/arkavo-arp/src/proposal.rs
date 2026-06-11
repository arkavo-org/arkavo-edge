//! Tightening proposals (§18, ARP v0.2): typed, evidence-backed requests to
//! make a policy stricter, produced by audit-plane components and reviewed
//! before application.
//!
//! The direction guarantee is structural: [`TighteningEffect`] is a closed
//! enum whose every variant restricts behavior — there are no loosen variants
//! to construct. Relaxation stays on the existing HITL relaxation path
//! (§15.1) and cannot ride this channel.

use serde::{Deserialize, Serialize};

use crate::constraints::{ApprovalMechanism, QuarantineScope};

/// How widely a proposal's effect propagates once applied. Ordering is the
/// declaration order and is load-bearing: auto-apply eligibility is
/// `blast_radius <= policy.auto_apply_max_blast_radius`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum BlastRadius {
    /// One adaptation entity (a single tool/model arm) on one agent.
    SingleEntity,
    /// One agent's whole ARP.
    SingleAgent,
    /// Every role ARP in one flight.
    Flight,
    /// Every agent in the mesh. Never auto-applied.
    Mesh,
}

/// Audit-plane component that produced a proposal.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProposalOrigin {
    /// Offline consolidation teacher (§7.4).
    Consolidation,
    /// Runtime critic.
    Critic,
    /// Historian / episode-audit role.
    Historian,
    /// Human operator.
    Human,
}

/// Lifecycle state. Transitions are owned by the runtime queue:
/// `proposed → reviewed → applied → observed → confirmed | reverted`,
/// with `rejected` reachable from `proposed`/`reviewed`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProposalState {
    Proposed,
    Reviewed,
    Applied,
    Observed,
    Confirmed,
    Reverted,
    Rejected,
}

/// Reference to the evidence behind a proposal: a DecisionTrace entry and,
/// when the evidence came from episodic memory, the episode store row ids.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRef {
    pub trace_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub episode_ids: Vec<String>,
}

/// Budget layer a [`TighteningEffect::LowerLayerBudget`] targets (§13.3).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BudgetLayerName {
    Cognitive,
    Execution,
    Data,
    Network,
    /// Offline learning spend (§7.4); wired by the consolidation teacher.
    Consolidation,
}

/// The typed effect of a tightening. Closed by construction: every variant
/// restricts behavior. Whether a value actually tightens *relative to the
/// current policy* (e.g. the new ceiling is below the present one) is checked
/// at ingest by the runtime, which knows the current values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TighteningEffect {
    /// Lower `budget.task_ceiling_usd`.
    LowerTaskCeiling { new_ceiling_usd: f64 },
    /// Lower one layer's limit in `budget.per_layer`.
    LowerLayerBudget {
        layer: BudgetLayerName,
        new_limit_usd: f64,
    },
    /// Raise `feedback_loops.immediate.quality_gate.threshold_default`.
    RaiseQualityGateThreshold { new_threshold: f64 },
    /// Quarantine a specific entity (§14.2).
    AddQuarantine {
        scope: QuarantineScope,
        entity: String,
    },
    /// Deny a tool outright.
    DenyTool { tool: String },
    /// Deny network egress to a host.
    DenyEgress { host: String },
    /// Lower a global rate limit (§13.4).
    LowerRateLimit { new_requests_per_second: f64 },
}

impl TighteningEffect {
    /// Value-sanity check for the effect, independent of current policy
    /// state. Returns the violation, or `None` when sane.
    #[must_use]
    pub fn value_violation(&self) -> Option<String> {
        match self {
            Self::LowerTaskCeiling { new_ceiling_usd } if *new_ceiling_usd <= 0.0 => Some(format!(
                "new_ceiling_usd must be positive, got {new_ceiling_usd}"
            )),
            Self::LowerLayerBudget { new_limit_usd, .. } if *new_limit_usd <= 0.0 => Some(format!(
                "new_limit_usd must be positive, got {new_limit_usd}"
            )),
            Self::RaiseQualityGateThreshold { new_threshold }
                if !(*new_threshold > 0.0 && *new_threshold <= 1.0) =>
            {
                Some(format!(
                    "new_threshold must be in (0, 1], got {new_threshold}"
                ))
            }
            Self::AddQuarantine { entity, .. } if entity.trim().is_empty() => {
                Some("quarantine entity must be non-empty".to_string())
            }
            Self::DenyTool { tool } if tool.trim().is_empty() => {
                Some("denied tool must be non-empty".to_string())
            }
            Self::DenyEgress { host } if host.trim().is_empty() => {
                Some("denied host must be non-empty".to_string())
            }
            Self::LowerRateLimit {
                new_requests_per_second,
            } if *new_requests_per_second <= 0.0 => Some(format!(
                "new_requests_per_second must be positive, got {new_requests_per_second}"
            )),
            _ => None,
        }
    }
}

/// An evidence-backed request to tighten policy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TighteningProposal {
    /// Unique proposal id (producer-assigned, typically a UUID string).
    pub id: String,
    pub origin: ProposalOrigin,
    pub state: ProposalState,
    pub effect: TighteningEffect,
    /// Plain-language rationale grounded in the evidence.
    pub rationale: String,
    pub blast_radius: BlastRadius,
    /// Must be non-empty: a proposal without evidence is rejected at ingest.
    pub evidence: Vec<TraceRef>,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// RFC 3339 timestamp set when the effect was applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_at: Option<String>,
    /// Identity that reviewed/approved (HITL path).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewed_by: Option<String>,
    /// Why the proposal was reverted or rejected, when it was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disposition_reason: Option<String>,
}

/// Per-document policy governing which proposals may be ingested and how far
/// they may auto-apply (§18.2). Lives at `ArpDocument.proposal_policy`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProposalPolicy {
    /// Origins this agent accepts proposals from. Must be non-empty.
    pub accepted_origins: Vec<ProposalOrigin>,
    /// Largest radius that may apply without human review.
    /// [`BlastRadius::Mesh`] is never auto-applied regardless of this value;
    /// validation rejects it here.
    pub auto_apply_max_blast_radius: BlastRadius,
    /// Review mechanism for proposals above the auto-apply radius (§15.1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<ApprovalMechanism>,
    /// How long an applied proposal stays in `observed` before it can be
    /// confirmed (no quality-gate regression attributable to the change).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation_window_sec: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[test]
    fn blast_radius_ordering_is_load_bearing() {
        assert!(BlastRadius::SingleEntity < BlastRadius::SingleAgent);
        assert!(BlastRadius::SingleAgent < BlastRadius::Flight);
        assert!(BlastRadius::Flight < BlastRadius::Mesh);
    }

    #[test]
    fn blast_radius_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&BlastRadius::SingleEntity).unwrap(),
            "\"single_entity\""
        );
        assert_eq!(
            serde_json::to_string(&BlastRadius::Mesh).unwrap(),
            "\"mesh\""
        );
    }

    #[test]
    fn effect_is_internally_tagged() {
        let effect = TighteningEffect::LowerTaskCeiling {
            new_ceiling_usd: 1.5,
        };
        let json = serde_json::to_value(&effect).unwrap();
        assert_eq!(json["kind"], "lower_task_ceiling");
        assert_eq!(json["new_ceiling_usd"], 1.5);

        let parsed: TighteningEffect = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, effect);
    }

    #[test]
    fn effect_value_sanity() {
        assert!(
            TighteningEffect::LowerTaskCeiling {
                new_ceiling_usd: -1.0
            }
            .value_violation()
            .is_some()
        );
        assert!(
            TighteningEffect::RaiseQualityGateThreshold { new_threshold: 1.2 }
                .value_violation()
                .is_some()
        );
        assert!(
            TighteningEffect::DenyTool { tool: "  ".into() }
                .value_violation()
                .is_some()
        );
        assert!(
            TighteningEffect::AddQuarantine {
                scope: QuarantineScope::Tool,
                entity: "game-rl:reset".into()
            }
            .value_violation()
            .is_none()
        );
    }

    #[test]
    fn proposal_round_trips() {
        let json = r#"{
            "id": "prop-1",
            "origin": "consolidation",
            "state": "proposed",
            "effect": {"kind": "deny_tool", "tool": "game-rl:reset"},
            "rationale": "reset correlated with colony loss in 4/4 failures",
            "blast_radius": "single_agent",
            "evidence": [{"trace_id": "t-1", "episode_ids": ["ep-1", "ep-2"]}],
            "created_at": "2026-06-10T00:00:00Z"
        }"#;
        let p: TighteningProposal = serde_json::from_str(json).unwrap();
        assert_eq!(p.origin, ProposalOrigin::Consolidation);
        assert_eq!(p.state, ProposalState::Proposed);
        assert_eq!(p.blast_radius, BlastRadius::SingleAgent);
        assert!(matches!(p.effect, TighteningEffect::DenyTool { .. }));
        let back = serde_json::to_string(&p).unwrap();
        let p2: TighteningProposal = serde_json::from_str(&back).unwrap();
        assert_eq!(p, p2);
    }

    #[spec("ARP-007")]
    #[test]
    fn there_is_no_loosen_variant() {
        // The closed-enum guarantee: deserializing a loosening "effect"
        // fails at the type level, not at a validation layer.
        let json = r#"{"kind": "raise_task_ceiling", "new_ceiling_usd": 100.0}"#;
        assert!(serde_json::from_str::<TighteningEffect>(json).is_err());
        let json = r#"{"kind": "allow_tool", "tool": "anything"}"#;
        assert!(serde_json::from_str::<TighteningEffect>(json).is_err());
    }
}
