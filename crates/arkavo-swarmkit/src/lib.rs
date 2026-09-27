//! SwarmKit manifest parser, canonical-form serializer, and validator.
//!
//! Provides typed Rust structs for deserializing SwarmKit manifests per
//! `swarmkit-spec-draft-00`, plus canonical-form JSON serialization,
//! BLAKE3 content addressing for `kit.id`, and cross-block validation
//! rules from §4.6, §5.1, §10.1.
//!
//! This crate is parser + data model + validator only. TDF encryption,
//! orchestrator delegation, A2A wiring, and specialist provisioning are
//! out of scope and live in downstream crates.

pub mod canonical;
pub mod coordination;
pub mod governance;
pub mod manifest;
pub mod pricing;
pub mod role;
pub mod skill_content;
pub mod validate;

pub use canonical::{canonical_json, content_hash, kit_id_for};
pub use coordination::{
    CompactionSpec, CompletionSpec, ConstraintsSpec, CoordinationSpec, EvaluationDimension,
    EvaluationRubric, EvaluationSpec, GlobalBudget, NetworkConstraints, OnFailure, ProvenanceSpec,
    Routing, Signature,
};
pub use governance::{
    ProposalApprovalMechanism, ProposalBlastRadius, ProposalGovernanceSpec, ProposalOriginSpec,
    RoleProposalCeiling,
};
pub use manifest::{Author, DeliverableSpec, InputSpec, KitMetadata, Manifest, Objective};
pub use pricing::ModelPricingEntry;
#[allow(deprecated)]
pub use role::ArpRule;
pub use role::{
    AgentProvisioning, AuthMode, Budget, ContextScope, Failure, Handoff, Inference, McpToolGrant,
    Model, Observability, Plane, RoleSpec, Skill, SkillSource, TdfAttributeReleasePolicy,
    TdfReleaseRule, ToolUse,
};
pub use skill_content::{SkillContent, SkillResource};
pub use validate::{ValidationError, validate};

/// Parse a SwarmKit manifest from a JSON string and run cross-block validation.
pub fn parse_json(json: &str) -> Result<Manifest, ParseError> {
    let manifest: Manifest = serde_json::from_str(json).map_err(ParseError::Json)?;
    validate(&manifest)?;
    Ok(manifest)
}

/// Parse a SwarmKit manifest from a YAML string and run cross-block validation.
pub fn parse_yaml(yaml: &str) -> Result<Manifest, ParseError> {
    let manifest: Manifest = serde_yaml::from_str(yaml).map_err(ParseError::Yaml)?;
    validate(&manifest)?;
    Ok(manifest)
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("validation error: {0}")]
    Validation(#[from] ValidationError),
}
