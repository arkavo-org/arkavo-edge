//! The manifest every kit producer starts from: `kit init` builds a whole
//! single-role kit from it, and `kit migrate-from-agents-md` reuses its
//! skeleton and role defaults around the roles it converts.

use base64::Engine;
use rand::Rng;

use arkavo_swarmkit::coordination::{RoutingStrategy, Topology};
use arkavo_swarmkit::role::{Isolation, Sandbox};
use arkavo_swarmkit::{
    AgentProvisioning, Author, Budget, CompletionSpec, ConstraintsSpec, CoordinationSpec,
    GlobalBudget, KitMetadata, KitRuntimeConfig, Manifest, Model, NetworkConstraints, Objective,
    OnFailure, ProvenanceSpec, RoleSpec, Routing, Signature, Skill, SkillSource,
};

/// Default single-role goal text; shared fallback `objective.goal` for `kit_build`.
pub(super) const DEFAULT_GOAL: &str =
    "Introduce yourself and assist with the tasks the user brings to you";
/// Default identity-skill instructions; shared fallback per-role instructions for `kit_build`.
pub(super) const DEFAULT_IDENTITY_INSTRUCTIONS: &str =
    "You are a helpful agent. Introduce yourself and assist with the tasks the user brings to you.";

pub(super) fn build_manifest(name: &str) -> Manifest {
    manifest_skeleton(
        name,
        format!("Locally authored single-role agent kit for {name}"),
        Objective {
            goal: DEFAULT_GOAL.to_string(),
            success_criteria: vec![
                "responds helpfully and stays within its configured budget".to_string(),
            ],
        },
        vec![primary_role()],
        Some(KitRuntimeConfig {
            local_dev: Some(true),
            mdns: Some(true),
            ..Default::default()
        }),
    )
}

/// The kit-metadata / coordination / constraints / completion / provenance
/// skeleton shared by every producer path (`kit init`, `kit
/// migrate-from-agents-md`). Producers own `objective` and `roles`; `runtime`
/// is optional since not every caller sets it.
pub(super) fn manifest_skeleton(
    name: &str,
    description: String,
    objective: Objective,
    roles: Vec<RoleSpec>,
    runtime: Option<KitRuntimeConfig>,
) -> Manifest {
    let now = chrono::Utc::now();
    let created = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let expires =
        (now + chrono::Duration::days(90)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let author_did = "did:web:local.arkavo.dev".to_string();

    Manifest {
        spec_version: "1.0.0".to_string(),
        kit: KitMetadata {
            id: String::new(),
            name: name.to_string(),
            version: "0.1.0".to_string(),
            description: Some(description),
            authors: vec![Author {
                did: author_did.clone(),
                name: Some("Local Author".to_string()),
            }],
            created,
            expires: Some(expires),
            nonce: generate_nonce(),
        },
        objective,
        inputs: vec![],
        deliverables: vec![],
        roles,
        coordination: CoordinationSpec {
            topology: Topology::HubSpoke,
            protocol: "a2a-jsonrpc-2.0".to_string(),
            routing: Routing {
                strategy: RoutingStrategy::Static,
                parameters: None,
            },
            context_sharing: None,
        },
        constraints: ConstraintsSpec {
            global_budget: GlobalBudget {
                max_wallclock_seconds: 300,
                max_total_tokens: 100_000,
                max_cost_usd: 0.50,
            },
            data_classifications: vec!["public".to_string()],
            jurisdiction: vec![],
            network: NetworkConstraints {
                egress_allowed: false,
                egress_allowlist: vec![],
            },
        },
        pricing: vec![],
        evaluation: None,
        proposal_governance: None,
        runtime,
        completion: CompletionSpec {
            rules: vec!["objective addressed".to_string()],
            on_failure: OnFailure::Abort,
            max_retries: 0,
        },
        provenance: ProvenanceSpec {
            c2pa_assertions: vec![],
            signatures: vec![Signature {
                signer_did: author_did,
                algorithm: "ed25519".to_string(),
                signature: "AAA".to_string(),
            }],
        },
    }
}

fn primary_role() -> RoleSpec {
    RoleSpec {
        id: "agent".to_string(),
        role_type: "operator".to_string(),
        plane: None,
        description: Some("Primary agent".to_string()),
        agent_provisioning: AgentProvisioning {
            model: Some(default_model()),
            inference: None,
            budget: Some(default_budget()),
            tool_use: None,
            context: None,
            observability: None,
            isolation: Some(default_isolation()),
            failure: None,
        },
        skills: vec![identity_skill(DEFAULT_IDENTITY_INSTRUCTIONS)],
        mcp_tools: vec![],
        tdf_attribute_release_policy: None,
        handoffs: vec![],
        context_scope: None,
    }
}

/// Default local edge model. Also the migrate-from-agents-md fallback for
/// unmapped or absent `model:` hints (brief item 4).
pub(super) fn default_model() -> Model {
    Model {
        family: "ministral".to_string(),
        size: Some("3B".to_string()),
        quantization: None,
        backend: Some("llama.cpp".to_string()),
        fallback: None,
    }
}

pub(super) fn default_budget() -> Budget {
    Budget {
        max_inference_calls: Some(32),
        max_wallclock_ms: None,
        max_total_tokens: Some(100_000),
    }
}

pub(super) fn default_isolation() -> Isolation {
    Isolation {
        sandbox: Some(Sandbox::Process),
        fs_writable: vec![],
        network_egress: Some(false),
    }
}

pub(super) fn identity_skill(instructions: &str) -> Skill {
    Skill {
        id: "skill:identity".to_string(),
        version: "0.1.0".to_string(),
        source: SkillSource::Inline,
        payload: Some(serde_json::json!({
            "name": "identity",
            "description": "System identity",
            "instructions": instructions,
            "resources": [],
        })),
        signature: None,
        signed_by: None,
    }
}

/// Replay-prevention nonce (spec §4.1): 16 random bytes, base64url no-pad.
fn generate_nonce() -> String {
    let bytes: [u8; 16] = rand::thread_rng().r#gen();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_swarmkit::validate;

    #[test]
    fn build_manifest_validates_with_empty_id() {
        let manifest = build_manifest("unit-test-agent");
        validate(&manifest).expect("freshly built manifest should validate before id assignment");
    }

    #[test]
    fn generate_nonce_is_nonempty_and_varies() {
        let a = generate_nonce();
        let b = generate_nonce();
        assert!(!a.is_empty());
        assert_ne!(a, b, "nonces should be randomly generated");
    }
}
