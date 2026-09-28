//! Top-level SwarmKit manifest types per spec §4.1, §4.2, §4.3 (kit + objective + io).

use serde::{Deserialize, Serialize};

use crate::coordination::{
    CompletionSpec, ConstraintsSpec, CoordinationSpec, EvaluationSpec, ProvenanceSpec,
};
use crate::governance::ProposalGovernanceSpec;
use crate::role::RoleSpec;
use crate::runtime_config::KitRuntimeConfig;

/// Top-level SwarmKit manifest. Cleartext document inside the TDF payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub spec_version: String,
    pub kit: KitMetadata,
    pub objective: Objective,
    #[serde(default)]
    pub inputs: Vec<InputSpec>,
    #[serde(default)]
    pub deliverables: Vec<DeliverableSpec>,
    pub roles: Vec<RoleSpec>,
    pub coordination: CoordinationSpec,
    pub constraints: ConstraintsSpec,
    /// Manifest-level model pricing table (cents per 1M tokens), authored at
    /// SwarmKit authoring time. Read at runtime to populate the budget cost
    /// model; never fetched from a vendor endpoint. Optional and skipped when
    /// empty, so a manifest without prices keeps its existing `kit.id`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pricing: Vec<crate::pricing::ModelPricingEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation: Option<EvaluationSpec>,
    /// Kit-level tightening-proposal governance. Optional: absent means no
    /// derived proposal policy (roles ingest nothing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal_governance: Option<ProposalGovernanceSpec>,
    /// Process/network/spend/preflight config (replaces product AGENTS.md).
    /// Optional: omitted kits keep historical kit.id hashes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<KitRuntimeConfig>,
    pub completion: CompletionSpec,
    pub provenance: ProvenanceSpec,
}

impl Manifest {
    /// Compute the canonical `kit.id` (BLAKE3 of the canonical manifest
    /// minus `kit.id` and `provenance.signatures`) and assign it to
    /// `self.kit.id`. The producer flow is:
    ///
    /// 1. Author the manifest with `kit.id: ""`.
    /// 2. Call `validate` (which skips the BLAKE3 round-trip when id is empty).
    /// 3. Call `compute_kit_id` to populate the field.
    /// 4. Optionally re-validate to confirm the round-trip holds.
    pub fn compute_kit_id(&mut self) -> Result<(), serde_json::Error> {
        let id = crate::canonical::kit_id_for(self)?;
        self.kit.id = id;
        Ok(())
    }
}

/// `kit` block per §4.1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KitMetadata {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub authors: Vec<Author>,
    pub created: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
    pub nonce: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Author {
    pub did: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Objective block per §4.1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Objective {
    pub goal: String,
    #[serde(default)]
    pub success_criteria: Vec<String>,
}

/// Input or deliverable spec per §4.2.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InputSpec {
    pub name: String,
    #[serde(rename = "type")]
    pub payload_type: PayloadType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
}

/// Deliverable spec — same shape as InputSpec minus `required`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeliverableSpec {
    pub name: String,
    #[serde(rename = "type")]
    pub payload_type: PayloadType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PayloadType {
    Text,
    Json,
    Binary,
    TdfRef,
    IrohTicket,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_type_round_trip() {
        let t = PayloadType::TdfRef;
        let s = serde_json::to_string(&t).unwrap();
        assert_eq!(s, "\"tdf-ref\"");
        let back: PayloadType = serde_json::from_str(&s).unwrap();
        assert_eq!(back, PayloadType::TdfRef);
    }

    #[test]
    fn objective_required_fields() {
        let json = r#"{"goal": "test goal"}"#;
        let obj: Objective = serde_json::from_str(json).unwrap();
        assert_eq!(obj.goal, "test goal");
        assert!(obj.success_criteria.is_empty());
    }

    #[arkavo_test_macros::spec("SK-017")]
    #[test]
    fn custom_did_methods_parse_as_opaque() {
        let json = r#"[
            {"did": "did:web:arkavo.com", "name": "Web identity"},
            {"did": "did:key:z6MkfTeBJZNQUNhNuRHmwCqKMs8oP1y3WX5z2Ai93iyz5XuC"},
            {"did": "did:plc:abcdefghijklmnop"}
        ]"#;
        let authors: Vec<Author> = serde_json::from_str(json).unwrap();
        assert_eq!(authors.len(), 3);
        assert_eq!(authors[0].did, "did:web:arkavo.com");
        assert!(authors[1].did.starts_with("did:key:"));
        assert!(authors[2].did.starts_with("did:plc:"));
    }
}
