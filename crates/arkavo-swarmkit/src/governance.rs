//! Kit-level tightening-proposal governance: how far audit-plane proposals
//! may auto-apply per role, and how flight-radius proposals are approved.
//!
//! These are SwarmKit spec types — deliberately independent of arkavo-arp
//! (this crate is parser + validator only). The runtime derives each role's
//! ARP `proposal_policy` from this block, the same way the global budget is
//! split per role.

use serde::{Deserialize, Serialize};

/// Blast radius vocabulary for governance ceilings. Ordering is declaration
/// order: `single_entity < single_agent < flight < mesh`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ProposalBlastRadius {
    SingleEntity,
    SingleAgent,
    Flight,
    Mesh,
}

/// Audit-plane origins a kit accepts proposals from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProposalOriginSpec {
    Consolidation,
    Critic,
    Historian,
    Human,
}

/// Approval mechanism for proposals above a role's auto-apply ceiling.
/// Mirrors ARP §15.1 `ApprovalMechanism`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProposalApprovalMechanism {
    CredentialPresentation,
    SignedCommand,
    InteractiveConfirmation,
}

/// Per-role governance ceiling.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoleProposalCeiling {
    /// Role id — must exist in the manifest's `roles`.
    pub role: String,
    /// Largest blast radius this role's ARP may auto-apply. `mesh` is never
    /// permitted as an auto-apply ceiling and fails validation.
    pub auto_apply_max_blast_radius: ProposalBlastRadius,
}

/// Kit-level proposal governance block.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProposalGovernanceSpec {
    /// Origins every role in this kit accepts proposals from. Empty means
    /// the conservative default (human only) at derivation time.
    #[serde(default)]
    pub accepted_origins: Vec<ProposalOriginSpec>,
    /// Per-role ceilings. Roles without an entry derive the default
    /// (`single_entity`).
    #[serde(default)]
    pub role_ceilings: Vec<RoleProposalCeiling>,
    /// Approval mechanism for proposals above a role's auto-apply ceiling —
    /// notably flight-radius proposals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flight_approval: Option<ProposalApprovalMechanism>,
    /// Observation window applied to derived proposal policies, seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation_window_sec: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blast_radius_ordering_and_wire_format() {
        assert!(ProposalBlastRadius::SingleEntity < ProposalBlastRadius::Mesh);
        assert_eq!(
            serde_json::to_string(&ProposalBlastRadius::SingleAgent).unwrap(),
            "\"single_agent\""
        );
    }

    #[test]
    fn governance_round_trips() {
        let json = r#"{
            "accepted_origins": ["consolidation", "human"],
            "role_ceilings": [
                {"role": "commander", "auto_apply_max_blast_radius": "single_agent"},
                {"role": "historian", "auto_apply_max_blast_radius": "single_entity"}
            ],
            "flight_approval": "interactive_confirmation",
            "observation_window_sec": 3600
        }"#;
        let g: ProposalGovernanceSpec = serde_json::from_str(json).unwrap();
        assert_eq!(g.accepted_origins.len(), 2);
        assert_eq!(g.role_ceilings.len(), 2);
        assert_eq!(
            g.flight_approval,
            Some(ProposalApprovalMechanism::InteractiveConfirmation)
        );
        let back = serde_json::to_string(&g).unwrap();
        let g2: ProposalGovernanceSpec = serde_json::from_str(&back).unwrap();
        assert_eq!(g, g2);
    }

    #[test]
    fn minimal_governance_defaults() {
        let g: ProposalGovernanceSpec = serde_json::from_str("{}").unwrap();
        assert!(g.accepted_origins.is_empty());
        assert!(g.role_ceilings.is_empty());
        assert!(g.flight_approval.is_none());
    }
}
