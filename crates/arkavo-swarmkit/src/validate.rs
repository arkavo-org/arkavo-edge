//! Cross-block validation rules per spec §4.6 (rubric weights), §5.1 (per-role caps),
//! §10.1 (expiry cap), §9.1 (kit.id matches BLAKE3 of canonical form).
//!
//! These rules cannot be expressed in the JSON Schema; they require relational
//! checks across blocks. The orchestrator MUST enforce them before provisioning.

use std::collections::HashSet;

use chrono::{DateTime, FixedOffset};

use crate::canonical::kit_id_for;
use crate::manifest::Manifest;

/// Maximum accepted kit lifetime per spec §10.1 — `expires - created` MUST be ≤ 1 year.
pub const MAX_EXPIRY_HORIZON_SECONDS: i64 = 365 * 24 * 60 * 60;
/// Recommended cap per spec §10.1 — SHOULD be ≤ 90 days.
pub const RECOMMENDED_EXPIRY_HORIZON_SECONDS: i64 = 90 * 24 * 60 * 60;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ValidationError {
    #[error("spec_version {found:?} is not a valid semver: {reason}")]
    SpecVersionUnparseable { found: String, reason: String },

    #[error("spec_version {found} unsupported; only 1.x is recognized")]
    SpecVersionMismatch { found: String },

    #[error("roles is empty; at least one role is required (§4.1)")]
    NoRoles,

    #[error("duplicate role id {0:?}")]
    DuplicateRoleId(String),

    #[error("handoff target {target:?} from role {from:?} does not resolve to a known role")]
    UnresolvedHandoff { from: String, target: String },

    #[error("evaluation.critic_role {0:?} does not resolve to a known role")]
    UnresolvedCriticRole(String),

    #[error(
        "agent_provisioning.failure.fallback_role {target:?} on role {from:?} does not resolve to a known role"
    )]
    UnresolvedFallbackRole { from: String, target: String },

    #[error(
        "agent_provisioning.budget.{field} on role {role:?} ({value}) exceeds constraints.global_budget.{field} ({limit})"
    )]
    BudgetExceedsGlobal {
        role: String,
        field: &'static str,
        value: u64,
        limit: u64,
    },

    #[error(
        "role {role:?} sets isolation.network_egress=true but constraints.network.egress_allowed=false"
    )]
    NetworkEgressDenied { role: String },

    #[error("evaluation.rubric.dimensions is empty; at least one dimension is required (§4.6)")]
    EmptyRubricDimensions,

    #[error(
        "proposal_governance.role_ceilings references role {0:?} which does not resolve to a known role"
    )]
    UnresolvedGovernanceRole(String),

    #[error(
        "proposal_governance.role_ceilings: role {role:?} sets auto_apply_max_blast_radius=mesh; mesh-radius proposals are never auto-applied"
    )]
    MeshAutoApplyCeiling { role: String },

    #[error(
        "role {role:?} declares plane={plane} but has coordination outputs ({output}); learning/audit roles may emit only tightening proposals and provenance-tagged prior observations"
    )]
    PlaneWiringViolation {
        role: String,
        plane: &'static str,
        output: &'static str,
    },

    #[error("evaluation.rubric.dimensions weight sum {sum} is not 1.0 (within ±1e-6)")]
    RubricWeightsDoNotSumToOne { sum: f64 },

    #[error("invalid timestamp {field:?}: {raw:?}")]
    InvalidTimestamp { field: &'static str, raw: String },

    #[error("kit.expires - kit.created exceeds the §10.1 1-year cap")]
    ExpiryHorizonTooLarge,

    #[error("kit.expires is at or before kit.created")]
    ExpiryBeforeCreated,

    #[error("kit.id {found:?} does not match BLAKE3 of canonical manifest (expected {expected:?})")]
    KitIdHashMismatch { found: String, expected: String },

    #[error("manifest could not be canonicalized for kit.id verification: {0}")]
    CanonicalSerializationFailed(String),
}

/// Validate the cross-block invariants of a SwarmKit manifest.
///
/// Returns the first encountered error. Skips `kit.id` verification when
/// `kit.id` is the empty string — producers building a new manifest can
/// author with an empty id, run `validate`, then assign the computed
/// hash via `Manifest::compute_kit_id` (or the lower-level
/// `canonical::kit_id_for`) and re-validate.
pub fn validate(m: &Manifest) -> Result<(), ValidationError> {
    let parsed = semver::Version::parse(&m.spec_version).map_err(|e| {
        ValidationError::SpecVersionUnparseable {
            found: m.spec_version.clone(),
            reason: e.to_string(),
        }
    })?;
    if parsed.major != 1 {
        return Err(ValidationError::SpecVersionMismatch {
            found: m.spec_version.clone(),
        });
    }

    if m.roles.is_empty() {
        return Err(ValidationError::NoRoles);
    }

    let mut role_ids: HashSet<&str> = HashSet::new();
    for role in &m.roles {
        if !role_ids.insert(&role.id) {
            return Err(ValidationError::DuplicateRoleId(role.id.clone()));
        }
    }

    for role in &m.roles {
        for h in &role.handoffs {
            if !role_ids.contains(h.to.as_str()) {
                return Err(ValidationError::UnresolvedHandoff {
                    from: role.id.clone(),
                    target: h.to.clone(),
                });
            }
        }
        if let Some(failure) = &role.agent_provisioning.failure
            && let Some(target) = &failure.fallback_role
            && !role_ids.contains(target.as_str())
        {
            return Err(ValidationError::UnresolvedFallbackRole {
                from: role.id.clone(),
                target: target.clone(),
            });
        }
        validate_role_budget(role, &m.constraints.global_budget)?;
        validate_network_egress(role, m.constraints.network.egress_allowed)?;
        validate_plane_wiring(role)?;
    }

    if let Some(eval) = &m.evaluation {
        if !role_ids.contains(eval.critic_role.as_str()) {
            return Err(ValidationError::UnresolvedCriticRole(
                eval.critic_role.clone(),
            ));
        }
        if eval.rubric.dimensions.is_empty() {
            return Err(ValidationError::EmptyRubricDimensions);
        }
        let sum: f64 = eval.rubric.dimensions.iter().map(|d| d.weight).sum();
        if (sum - 1.0).abs() > 1e-6 {
            return Err(ValidationError::RubricWeightsDoNotSumToOne { sum });
        }
    }

    validate_proposal_governance(m, &role_ids)?;

    validate_expiry(m)?;

    if !m.kit.id.is_empty() {
        validate_kit_id(m)?;
    }

    Ok(())
}

/// Learning/audit roles must not have coordination outputs: no handoff
/// delegation, no shared-context writes. The type-level half of "Fable on
/// the audit plane, never the coordination plane".
fn validate_plane_wiring(role: &crate::role::RoleSpec) -> Result<(), ValidationError> {
    use crate::role::Plane;
    let plane = match role.plane {
        Some(Plane::Learning) => "learning",
        Some(Plane::Audit) => "audit",
        Some(Plane::Coordination) | None => return Ok(()),
    };
    if !role.handoffs.is_empty() {
        return Err(ValidationError::PlaneWiringViolation {
            role: role.id.clone(),
            plane,
            output: "handoffs (A2A delegation)",
        });
    }
    if let Some(scope) = &role.context_scope
        && !scope.can_write.is_empty()
    {
        return Err(ValidationError::PlaneWiringViolation {
            role: role.id.clone(),
            plane,
            output: "context_scope.can_write",
        });
    }
    Ok(())
}

fn validate_proposal_governance(
    m: &Manifest,
    role_ids: &HashSet<&str>,
) -> Result<(), ValidationError> {
    if let Some(gov) = &m.proposal_governance {
        for ceiling in &gov.role_ceilings {
            if !role_ids.contains(ceiling.role.as_str()) {
                return Err(ValidationError::UnresolvedGovernanceRole(
                    ceiling.role.clone(),
                ));
            }
            if ceiling.auto_apply_max_blast_radius == crate::governance::ProposalBlastRadius::Mesh {
                return Err(ValidationError::MeshAutoApplyCeiling {
                    role: ceiling.role.clone(),
                });
            }
        }
    }
    Ok(())
}

fn validate_role_budget(
    role: &crate::role::RoleSpec,
    global: &crate::coordination::GlobalBudget,
) -> Result<(), ValidationError> {
    let Some(budget) = &role.agent_provisioning.budget else {
        return Ok(());
    };

    let role_id = &role.id;

    if let Some(ms) = budget.max_wallclock_ms {
        let global_ms = global.max_wallclock_seconds.saturating_mul(1000);
        if ms > global_ms {
            return Err(ValidationError::BudgetExceedsGlobal {
                role: role_id.clone(),
                field: "max_wallclock_ms",
                value: ms,
                limit: global_ms,
            });
        }
    }

    if let Some(toks) = budget.max_total_tokens
        && toks > global.max_total_tokens
    {
        return Err(ValidationError::BudgetExceedsGlobal {
            role: role_id.clone(),
            field: "max_total_tokens",
            value: toks,
            limit: global.max_total_tokens,
        });
    }

    Ok(())
}

fn validate_network_egress(
    role: &crate::role::RoleSpec,
    global_allowed: bool,
) -> Result<(), ValidationError> {
    let Some(isolation) = &role.agent_provisioning.isolation else {
        return Ok(());
    };
    if isolation.network_egress == Some(true) && !global_allowed {
        return Err(ValidationError::NetworkEgressDenied {
            role: role.id.clone(),
        });
    }
    Ok(())
}

fn validate_expiry(m: &Manifest) -> Result<(), ValidationError> {
    let Some(expires_str) = &m.kit.expires else {
        return Ok(());
    };
    let created = parse_rfc3339(&m.kit.created, "kit.created")?;
    let expires = parse_rfc3339(expires_str, "kit.expires")?;
    let delta = expires.signed_duration_since(created).num_seconds();
    if delta <= 0 {
        return Err(ValidationError::ExpiryBeforeCreated);
    }
    if delta > MAX_EXPIRY_HORIZON_SECONDS {
        return Err(ValidationError::ExpiryHorizonTooLarge);
    }
    Ok(())
}

fn parse_rfc3339(raw: &str, field: &'static str) -> Result<DateTime<FixedOffset>, ValidationError> {
    DateTime::parse_from_rfc3339(raw).map_err(|_| ValidationError::InvalidTimestamp {
        field,
        raw: raw.to_string(),
    })
}

fn validate_kit_id(m: &Manifest) -> Result<(), ValidationError> {
    let expected = kit_id_for(m).map_err(|e| {
        // Distinct from KitIdHashMismatch: a serialization failure on a
        // successfully-deserialized Manifest is a bug, not a producer
        // error. Surface the underlying serde_json message for diagnosis.
        ValidationError::CanonicalSerializationFailed(e.to_string())
    })?;
    if expected != m.kit.id {
        return Err(ValidationError::KitIdHashMismatch {
            found: m.kit.id.clone(),
            expected,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::*;
    use crate::manifest::*;
    use crate::role::*;
    use arkavo_test_macros::spec;

    fn minimal_manifest() -> Manifest {
        Manifest {
            spec_version: "1.0.0".into(),
            kit: KitMetadata {
                id: String::new(),
                name: "test".into(),
                version: "1.0.0".into(),
                description: None,
                authors: vec![Author {
                    did: "did:web:example.com".into(),
                    name: None,
                }],
                created: "2026-04-29T00:00:00Z".into(),
                expires: Some("2026-05-29T00:00:00Z".into()),
                nonce: "thz1Cz8aWOUURbyQQfvA0Q".into(),
            },
            objective: Objective {
                goal: "test".into(),
                success_criteria: vec![],
            },
            inputs: vec![],
            deliverables: vec![],
            roles: vec![RoleSpec {
                id: "r1".into(),
                role_type: "specialist".into(),
                plane: None,
                description: None,
                agent_provisioning: AgentProvisioning::default(),
                skills: vec![],
                mcp_tools: vec![],
                tdf_attribute_release_policy: None,
                handoffs: vec![],
                context_scope: None,
            }],
            coordination: CoordinationSpec {
                topology: Topology::HubSpoke,
                protocol: "a2a-jsonrpc-2.0".into(),
                routing: Routing {
                    strategy: RoutingStrategy::Static,
                    parameters: None,
                },
                context_sharing: None,
            },
            constraints: ConstraintsSpec {
                global_budget: GlobalBudget {
                    max_wallclock_seconds: 60,
                    max_total_tokens: 8000,
                    max_cost_usd: 0.01,
                },
                data_classifications: vec!["public".into()],
                jurisdiction: vec![],
                network: NetworkConstraints {
                    egress_allowed: false,
                    egress_allowlist: vec![],
                },
            },
            evaluation: None,
            proposal_governance: None,
            completion: CompletionSpec {
                rules: vec!["all deliverables present".into()],
                on_failure: OnFailure::Abort,
                max_retries: 0,
            },
            provenance: ProvenanceSpec {
                c2pa_assertions: vec![],
                signatures: vec![Signature {
                    signer_did: "did:web:example.com".into(),
                    algorithm: "ed25519".into(),
                    signature: "AAA".into(),
                }],
            },
        }
    }

    #[test]
    fn minimal_validates() {
        validate(&minimal_manifest()).unwrap();
    }

    #[spec("SK-002")]
    #[test]
    fn budget_overflow_caught() {
        let mut m = minimal_manifest();
        m.roles[0].agent_provisioning.budget = Some(Budget {
            max_total_tokens: Some(99_999),
            ..Default::default()
        });
        let err = validate(&m).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::BudgetExceedsGlobal {
                field: "max_total_tokens",
                ..
            }
        ));
    }

    #[test]
    fn network_egress_denied() {
        let mut m = minimal_manifest();
        m.roles[0].agent_provisioning.isolation = Some(Isolation {
            sandbox: None,
            fs_writable: vec![],
            network_egress: Some(true),
        });
        let err = validate(&m).unwrap_err();
        assert!(matches!(err, ValidationError::NetworkEgressDenied { .. }));
    }

    #[test]
    fn unresolved_handoff() {
        let mut m = minimal_manifest();
        m.roles[0].handoffs.push(Handoff {
            to: "ghost".into(),
            on: "always".into(),
        });
        let err = validate(&m).unwrap_err();
        assert!(matches!(err, ValidationError::UnresolvedHandoff { .. }));
    }

    #[test]
    fn unresolved_critic() {
        let mut m = minimal_manifest();
        m.evaluation = Some(EvaluationSpec {
            rubric: EvaluationRubric {
                reference: None,
                dimensions: vec![EvaluationDimension {
                    name: "accuracy".into(),
                    weight: 1.0,
                    threshold: 0.7,
                }],
            },
            critic_role: "ghost".into(),
            sample_size: None,
        });
        let err = validate(&m).unwrap_err();
        assert!(matches!(err, ValidationError::UnresolvedCriticRole(_)));
    }

    #[spec("SK-005")]
    #[test]
    fn rubric_weights_must_sum_to_one() {
        let mut m = minimal_manifest();
        m.evaluation = Some(EvaluationSpec {
            rubric: EvaluationRubric {
                reference: None,
                dimensions: vec![
                    EvaluationDimension {
                        name: "a".into(),
                        weight: 0.4,
                        threshold: 0.5,
                    },
                    EvaluationDimension {
                        name: "b".into(),
                        weight: 0.4,
                        threshold: 0.5,
                    },
                ],
            },
            critic_role: "r1".into(),
            sample_size: None,
        });
        let err = validate(&m).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::RubricWeightsDoNotSumToOne { .. }
        ));
    }

    #[spec("SK-004")]
    #[test]
    fn expiry_horizon_capped() {
        let mut m = minimal_manifest();
        m.kit.expires = Some("2030-01-01T00:00:00Z".into()); // > 1 year out
        let err = validate(&m).unwrap_err();
        assert_eq!(err, ValidationError::ExpiryHorizonTooLarge);
    }

    #[spec("SK-004")]
    #[test]
    fn expiry_before_created() {
        let mut m = minimal_manifest();
        m.kit.expires = Some("2026-04-28T00:00:00Z".into()); // before created
        let err = validate(&m).unwrap_err();
        assert_eq!(err, ValidationError::ExpiryBeforeCreated);
    }

    #[test]
    fn duplicate_role_id() {
        let mut m = minimal_manifest();
        m.roles.push(m.roles[0].clone());
        let err = validate(&m).unwrap_err();
        assert!(matches!(err, ValidationError::DuplicateRoleId(_)));
    }

    #[spec("SK-003")]
    #[test]
    fn kit_id_round_trip() {
        let mut m = minimal_manifest();
        let id = crate::canonical::kit_id_for(&m).unwrap();
        m.kit.id = id;
        validate(&m).unwrap();
    }

    #[spec("SK-003")]
    #[test]
    fn kit_id_mismatch_detected() {
        let mut m = minimal_manifest();
        m.kit.id = "blake3:wrong".into();
        let err = validate(&m).unwrap_err();
        assert!(matches!(err, ValidationError::KitIdHashMismatch { .. }));
    }

    #[spec("SK-007")]
    #[test]
    fn custom_rubric_dimension_names_accepted() {
        let mut m = minimal_manifest();
        m.evaluation = Some(EvaluationSpec {
            rubric: EvaluationRubric {
                reference: None,
                dimensions: vec![
                    EvaluationDimension {
                        name: "spec_compliance".into(),
                        weight: 0.4,
                        threshold: 0.95,
                    },
                    EvaluationDimension {
                        name: "prompt_fidelity".into(),
                        weight: 0.3,
                        threshold: 0.7,
                    },
                    EvaluationDimension {
                        name: "accessibility".into(),
                        weight: 0.2,
                        threshold: 0.7,
                    },
                    EvaluationDimension {
                        name: "performance_hints".into(),
                        weight: 0.1,
                        threshold: 0.7,
                    },
                ],
            },
            critic_role: "r1".into(),
            sample_size: None,
        });
        validate(&m).expect("custom dimension names should validate");
    }

    #[spec("SK-008")]
    #[test]
    fn custom_tdf_attribute_strings_accepted() {
        let mut m = minimal_manifest();
        m.roles[0].tdf_attribute_release_policy = Some(TdfAttributeReleasePolicy {
            attributes: vec![
                "https://attr.arkavo.com/role/auditor".into(),
                "https://attr.arkavo.com/clearance/restricted".into(),
                "https://attr.arkavo.com/jurisdiction/us-ca".into(),
                "https://attr.arkavo.com/audit_authority/true".into(),
            ],
            rule: TdfReleaseRule::AllOf,
        });
        validate(&m).expect("custom attribute strings should validate");
        let attrs = &m.roles[0]
            .tdf_attribute_release_policy
            .as_ref()
            .unwrap()
            .attributes;
        assert_eq!(attrs.len(), 4);
        assert!(attrs.iter().any(|a| a.ends_with("/audit_authority/true")));
    }

    #[spec("SK-009")]
    #[test]
    fn custom_data_classifications_accepted() {
        let mut m = minimal_manifest();
        m.constraints.data_classifications = vec![
            "restricted".into(),
            "tactical".into(),
            "domain-specific".into(),
        ];
        validate(&m).expect("custom data_classifications should validate");
        assert_eq!(m.constraints.data_classifications.len(), 3);
    }
    #[test]
    fn governance_with_known_roles_validates() {
        use crate::governance::{
            ProposalBlastRadius, ProposalGovernanceSpec, ProposalOriginSpec, RoleProposalCeiling,
        };
        let mut m = minimal_manifest();
        m.proposal_governance = Some(ProposalGovernanceSpec {
            accepted_origins: vec![ProposalOriginSpec::Human],
            role_ceilings: vec![RoleProposalCeiling {
                role: "r1".into(),
                auto_apply_max_blast_radius: ProposalBlastRadius::SingleAgent,
            }],
            flight_approval: None,
            observation_window_sec: None,
        });
        assert!(validate(&m).is_ok());
    }

    #[test]
    fn governance_unknown_role_rejected() {
        use crate::governance::{ProposalBlastRadius, ProposalGovernanceSpec, RoleProposalCeiling};
        let mut m = minimal_manifest();
        m.proposal_governance = Some(ProposalGovernanceSpec {
            accepted_origins: vec![],
            role_ceilings: vec![RoleProposalCeiling {
                role: "ghost".into(),
                auto_apply_max_blast_radius: ProposalBlastRadius::SingleEntity,
            }],
            flight_approval: None,
            observation_window_sec: None,
        });
        assert!(matches!(
            validate(&m),
            Err(ValidationError::UnresolvedGovernanceRole(r)) if r == "ghost"
        ));
    }

    #[test]
    fn governance_mesh_auto_apply_rejected() {
        use crate::governance::{ProposalBlastRadius, ProposalGovernanceSpec, RoleProposalCeiling};
        let mut m = minimal_manifest();
        m.proposal_governance = Some(ProposalGovernanceSpec {
            accepted_origins: vec![],
            role_ceilings: vec![RoleProposalCeiling {
                role: "r1".into(),
                auto_apply_max_blast_radius: ProposalBlastRadius::Mesh,
            }],
            flight_approval: None,
            observation_window_sec: None,
        });
        assert!(matches!(
            validate(&m),
            Err(ValidationError::MeshAutoApplyCeiling { role }) if role == "r1"
        ));
    }
    #[test]
    fn audit_plane_role_with_handoffs_rejected() {
        let mut m = minimal_manifest();
        m.roles[0].plane = Some(crate::role::Plane::Audit);
        m.roles[0].handoffs = vec![Handoff {
            to: "r1".into(),
            on: "done".into(),
        }];
        assert!(matches!(
            validate(&m),
            Err(ValidationError::PlaneWiringViolation { plane: "audit", .. })
        ));
    }

    #[test]
    fn learning_plane_role_with_context_writes_rejected() {
        let mut m = minimal_manifest();
        m.roles[0].plane = Some(crate::role::Plane::Learning);
        m.roles[0].context_scope = Some(ContextScope {
            can_read: vec!["*".into()],
            can_write: vec!["lessons".into()],
        });
        assert!(matches!(
            validate(&m),
            Err(ValidationError::PlaneWiringViolation {
                plane: "learning",
                ..
            })
        ));
    }

    #[test]
    fn audit_plane_role_without_outputs_validates() {
        let mut m = minimal_manifest();
        m.roles[0].plane = Some(crate::role::Plane::Audit);
        m.roles[0].context_scope = Some(ContextScope {
            can_read: vec!["*".into()],
            can_write: vec![],
        });
        assert!(validate(&m).is_ok());
    }

    #[test]
    fn coordination_role_keeps_handoffs() {
        // Handoffs stay legal for coordination roles (declared or default).
        let mut m = minimal_manifest();
        m.roles[0].plane = Some(crate::role::Plane::Coordination);
        m.roles[0].handoffs = vec![Handoff {
            to: "r1".into(),
            on: "done".into(),
        }];
        assert!(validate(&m).is_ok());
    }
}
