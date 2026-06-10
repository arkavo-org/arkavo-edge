//! Capability proposals (§19, ARP v0.2): requests to **add** capability.
//!
//! A new capability is not a tightening and cannot ride the proposal
//! channel — [`crate::proposal::TighteningEffect`] is a closed enum with no
//! capability-addition variant, so the exclusion is structural, not policy.
//! This second class carries mandatory gates instead: every gate must pass
//! and a human must approve before a capability proposal is admissible.
//! There is deliberately no auto-apply field on this type at all.
//!
//! Two channels, asymmetric friction: tightenings flow, capabilities crawl.

use serde::{Deserialize, Serialize};

use crate::proposal::{ProposalOrigin, TraceRef};

/// What the proposed capability is: a WASM component behind a WIT contract,
/// gated by a torg-verified circuit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityRef {
    /// Capability name (becomes the tool/import name).
    pub name: String,
    /// WIT world the component implements (WASI 0.2 pinned for v1).
    pub wit_world: String,
    /// Content digest of the component binary (e.g. `blake3:<hex>`).
    pub component_digest: String,
    /// Digest of the torg gating circuit that decides whether/when the
    /// capability may run. The component decides only how.
    pub gating_circuit_digest: String,
}

/// The mandatory gate checklist. All gates must pass; none is optional and
/// none can be waived by configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityGates {
    /// sbe+sat verifier pass on the gating circuit: SAT boundary probing
    /// found no violation and the SBE invariant layer holds.
    pub verifier_pass: bool,
    /// The component conforms to its declared WIT world.
    pub wit_conformance: bool,
    /// SwarmKit conformance suite passed with the capability enabled.
    pub swarmkit_conformance: bool,
    /// Canary completed under an ARP Quarantine scope without incident.
    pub canary_completed: bool,
    /// Identity of the human approver. `None` means not approved — human
    /// approval is a recorded identity, never a boolean.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub human_approval: Option<String>,
    /// The kit was re-signed with the component as a member. Components
    /// execute only as members of a signed, TDF-wrapped kit.
    pub kit_resigned: bool,
}

impl CapabilityGates {
    /// Names of the gates still unpassed, in checklist order.
    #[must_use]
    pub fn remaining(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if !self.verifier_pass {
            out.push("verifier_pass (sbe+sat)");
        }
        if !self.wit_conformance {
            out.push("wit_conformance");
        }
        if !self.swarmkit_conformance {
            out.push("swarmkit_conformance");
        }
        if !self.canary_completed {
            out.push("canary_completed (quarantine scope)");
        }
        if self.human_approval.is_none() {
            out.push("human_approval");
        }
        if !self.kit_resigned {
            out.push("kit_resigned");
        }
        out
    }

    /// True only when every gate passed and a human approved.
    #[must_use]
    pub fn all_passed(&self) -> bool {
        self.remaining().is_empty()
    }
}

/// Lifecycle of a capability proposal. There is no auto-apply transition:
/// `Active` is reachable only through [`CapabilityGates::all_passed`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    Proposed,
    /// Gates in progress.
    Gated,
    /// Every gate passed, human approved — eligible for kit inclusion.
    Approved,
    /// Shipped as a member of a re-signed kit.
    Active,
    Withdrawn,
}

/// A request to add capability, carrying its gate checklist.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapabilityProposal {
    pub id: String,
    pub origin: ProposalOrigin,
    pub state: CapabilityState,
    pub capability: CapabilityRef,
    pub rationale: String,
    /// Must be non-empty, like every proposal in the system.
    pub evidence: Vec<TraceRef>,
    pub gates: CapabilityGates,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disposition_reason: Option<String>,
}

impl CapabilityProposal {
    /// Validation at ingest: evidence and capability identifiers must be
    /// present. Returns the violation, or `None` when admissible for gating.
    #[must_use]
    pub fn intake_violation(&self) -> Option<String> {
        if self.evidence.is_empty() {
            return Some(format!("capability proposal {} has no evidence", self.id));
        }
        for (field, value) in [
            ("name", &self.capability.name),
            ("wit_world", &self.capability.wit_world),
            ("component_digest", &self.capability.component_digest),
            (
                "gating_circuit_digest",
                &self.capability.gating_circuit_digest,
            ),
        ] {
            if value.trim().is_empty() {
                return Some(format!("capability.{field} must be non-empty"));
            }
        }
        None
    }

    /// Whether the proposal may transition to [`CapabilityState::Approved`].
    /// The only road to activation runs through every gate plus a named
    /// human approver.
    #[must_use]
    pub fn is_approvable(&self) -> bool {
        self.intake_violation().is_none() && self.gates.all_passed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proposal(gates: CapabilityGates) -> CapabilityProposal {
        CapabilityProposal {
            id: "cap-1".into(),
            origin: ProposalOrigin::Consolidation,
            state: CapabilityState::Gated,
            capability: CapabilityRef {
                name: "summarize_table".into(),
                wit_world: "arkavo:capability/tool@0.2.0".into(),
                component_digest: "blake3:abc123".into(),
                gating_circuit_digest: "blake3:def456".into(),
            },
            rationale: "repeated manual table summarization in episodes".into(),
            evidence: vec![TraceRef {
                trace_id: "t-1".into(),
                episode_ids: vec!["ep-1".into()],
            }],
            gates,
            created_at: "2026-06-10T00:00:00Z".into(),
            disposition_reason: None,
        }
    }

    fn all_gates() -> CapabilityGates {
        CapabilityGates {
            verifier_pass: true,
            wit_conformance: true,
            swarmkit_conformance: true,
            canary_completed: true,
            human_approval: Some("did:web:operator.example.com".into()),
            kit_resigned: true,
        }
    }

    #[test]
    fn default_gates_block_approval_with_full_checklist() {
        let p = proposal(CapabilityGates::default());
        assert!(!p.is_approvable());
        assert_eq!(p.gates.remaining().len(), 6);
    }

    #[test]
    fn every_gate_plus_named_human_is_approvable() {
        let p = proposal(all_gates());
        assert!(p.is_approvable());
        assert!(p.gates.remaining().is_empty());
    }

    #[test]
    fn any_single_missing_gate_blocks() {
        for missing in 0..6 {
            let mut gates = all_gates();
            match missing {
                0 => gates.verifier_pass = false,
                1 => gates.wit_conformance = false,
                2 => gates.swarmkit_conformance = false,
                3 => gates.canary_completed = false,
                4 => gates.human_approval = None,
                _ => gates.kit_resigned = false,
            }
            let p = proposal(gates);
            assert!(!p.is_approvable(), "gate {missing} must block approval");
            assert_eq!(p.gates.remaining().len(), 1);
        }
    }

    #[test]
    fn human_approval_is_an_identity_not_a_boolean() {
        let p = proposal(all_gates());
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(
            json["gates"]["human_approval"],
            "did:web:operator.example.com"
        );
    }

    #[test]
    fn intake_requires_evidence_and_identifiers() {
        let mut p = proposal(all_gates());
        p.evidence.clear();
        assert!(p.intake_violation().unwrap().contains("no evidence"));

        let mut p = proposal(all_gates());
        p.capability.component_digest = "  ".into();
        assert!(p.intake_violation().unwrap().contains("component_digest"));
    }

    #[test]
    fn capability_addition_cannot_ride_the_tightening_channel() {
        // The structural exclusion: there is no TighteningEffect variant
        // that adds capability, so a capability "effect" fails to parse.
        let json = r#"{"kind": "add_capability", "name": "summarize_table"}"#;
        assert!(serde_json::from_str::<crate::proposal::TighteningEffect>(json).is_err());
        let json = r#"{"kind": "allow_tool", "tool": "summarize_table"}"#;
        assert!(serde_json::from_str::<crate::proposal::TighteningEffect>(json).is_err());
    }
}
