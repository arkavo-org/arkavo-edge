//! Controls a manifest declares that `arkavo agent -c <kit>` does not act on.
//!
//! The CLI agent path builds one agent per role from the role's id, model
//! and skill instructions, plus the kit's `runtime` block. The rest of the
//! manifest parses and validates, which can read as if it were in force. This
//! module names the declared fields that have no consumer on that path so a
//! tool can tell the author before they rely on them.
//!
//! The list is scoped to that one entry point. Some of these fields are read
//! elsewhere (for example `SwarmFlight::launch` verifies skill signatures
//! when a resolver is configured); a field belongs here when nothing the
//! agent process starts reads it.

use crate::manifest::Manifest;
use crate::role::RoleSpec;

/// The entry point the report is about, for callers that label their output.
pub const AGENT_PATH: &str = "arkavo agent -c <kit>";

/// One declared control with no effect on the CLI agent path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnenforcedControl {
    /// Manifest path of the field, with `roles[]` standing for each role
    /// named in `roles`.
    pub field: &'static str,
    /// Ids of the roles that declare the field, in manifest order. Empty for
    /// a kit-level field.
    pub roles: Vec<String>,
    /// What is not done with the declared value.
    pub effect: &'static str,
}

struct RoleRule {
    field: &'static str,
    effect: &'static str,
    declared: fn(&RoleSpec) -> bool,
}

struct KitRule {
    field: &'static str,
    effect: &'static str,
    declared: fn(&Manifest) -> bool,
}

const ROLE_RULES: &[RoleRule] = &[
    RoleRule {
        field: "roles[].agent_provisioning.isolation",
        effect: "sandbox, fs_writable and network_egress are not applied to the agent process",
        declared: |r| r.agent_provisioning.isolation.is_some(),
    },
    RoleRule {
        field: "roles[].agent_provisioning.budget",
        effect: "max_inference_calls, max_wallclock_ms and max_total_tokens are not applied as limits",
        declared: |r| r.agent_provisioning.budget.is_some(),
    },
    RoleRule {
        field: "roles[].agent_provisioning.tool_use",
        effect: "max_calls, max_parallel, on_error and retry_policy are not applied",
        declared: |r| r.agent_provisioning.tool_use.is_some(),
    },
    RoleRule {
        field: "roles[].mcp_tools",
        effect: "grants are not used to restrict which tools the role can call",
        declared: |r| !r.mcp_tools.is_empty(),
    },
    RoleRule {
        field: "roles[].context_scope",
        effect: "can_read and can_write are not applied",
        declared: |r| r.context_scope.is_some(),
    },
    RoleRule {
        field: "roles[].tdf_attribute_release_policy",
        effect: "no TDF policy is built from the role's attributes",
        declared: |r| r.tdf_attribute_release_policy.is_some(),
    },
    RoleRule {
        field: "roles[].handoffs",
        effect: "handoff targets are not used for routing",
        declared: |r| !r.handoffs.is_empty(),
    },
    RoleRule {
        field: "roles[].skills[].signature",
        effect: "skill signatures are not verified; skill instructions are used as written",
        declared: |r| {
            r.skills
                .iter()
                .any(|s| s.signature.is_some() || s.signed_by.is_some())
        },
    },
];

const KIT_RULES: &[KitRule] = &[
    KitRule {
        field: "constraints.global_budget",
        effect: "max_wallclock_seconds, max_total_tokens and max_cost_usd are not applied as limits",
        declared: |_| true,
    },
    KitRule {
        field: "constraints.network",
        effect: "egress_allowed and egress_allowlist do not restrict network access",
        declared: |_| true,
    },
    KitRule {
        field: "constraints.data_classifications",
        effect: "classifications are not applied to data the agent handles",
        declared: |m| !m.constraints.data_classifications.is_empty(),
    },
    KitRule {
        field: "constraints.jurisdiction",
        effect: "jurisdictions are not applied",
        declared: |m| !m.constraints.jurisdiction.is_empty(),
    },
    KitRule {
        field: "evaluation",
        effect: "the rubric is not scored and critic_role is not consulted",
        declared: |m| m.evaluation.is_some(),
    },
    KitRule {
        field: "completion",
        effect: "rules, on_failure and max_retries are not evaluated",
        declared: |_| true,
    },
    KitRule {
        field: "provenance.signatures",
        effect: "manifest signatures are not verified",
        declared: |m| !m.provenance.signatures.is_empty(),
    },
    KitRule {
        field: "runtime.local_dev",
        effect: "has no effect; skill signatures are not verified for either value",
        declared: |m| m.runtime.as_ref().is_some_and(|r| r.local_dev.is_some()),
    },
];

/// The controls `m` declares that have no effect when the kit is run with
/// `arkavo agent -c <kit>`, role-level fields first, each in a fixed order.
///
/// A field is reported only when the manifest sets it, so the result
/// describes this kit and not the format. `constraints.global_budget`,
/// `constraints.network` and `completion` are required blocks and therefore
/// always reported.
pub fn unenforced_on_agent_path(m: &Manifest) -> Vec<UnenforcedControl> {
    let role_level = ROLE_RULES.iter().filter_map(|rule| {
        let roles: Vec<String> = m
            .roles
            .iter()
            .filter(|role| (rule.declared)(role))
            .map(|role| role.id.clone())
            .collect();
        (!roles.is_empty()).then_some(UnenforcedControl {
            field: rule.field,
            roles,
            effect: rule.effect,
        })
    });
    let kit_level = KIT_RULES
        .iter()
        .filter(|rule| (rule.declared)(m))
        .map(|rule| UnenforcedControl {
            field: rule.field,
            roles: Vec::new(),
            effect: rule.effect,
        });
    role_level.chain(kit_level).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_yaml;

    const BARE_KIT: &str = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "bare"
  version: "0.1.0"
  authors:
    - did: "did:web:example.com"
  created: "2026-04-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "say hello"
roles:
  - id: agent
    role_type: operator
    agent_provisioning: {}
coordination:
  topology: hub-spoke
  protocol: a2a-jsonrpc-2.0
  routing:
    strategy: static
constraints:
  global_budget:
    max_wallclock_seconds: 60
    max_total_tokens: 8000
    max_cost_usd: 0.01
  network:
    egress_allowed: false
completion:
  rules: ["done"]
  on_failure: abort
  max_retries: 0
provenance:
  signatures: []
"#;

    const FULL_KIT: &str = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "full"
  version: "0.1.0"
  authors:
    - did: "did:web:example.com"
  created: "2026-04-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "review"
roles:
  - id: planner
    role_type: planner
    agent_provisioning:
      budget: {max_total_tokens: 4000}
      isolation: {sandbox: process, network_egress: false}
      tool_use: {max_calls: 4}
    skills:
      - id: "skill:plan"
        version: "1.0.0"
        source: inline
        payload: {name: plan, description: d, instructions: "Plan.", resources: []}
        signature: "c2ln"
        signed_by: "did:web:example.com"
    mcp_tools:
      - server: files
        tools: [read_file]
        auth: none
    tdf_attribute_release_policy:
      attributes: ["https://example.com/attr/role/value/planner"]
      rule: allOf
    handoffs:
      - to: critic
        on: "plan ready"
    context_scope:
      can_read: ["plan"]
      can_write: ["plan"]
  - id: critic
    role_type: critic
    agent_provisioning:
      budget: {max_total_tokens: 2000}
coordination:
  topology: pipeline
  protocol: a2a-jsonrpc-2.0
  routing:
    strategy: static
constraints:
  global_budget:
    max_wallclock_seconds: 60
    max_total_tokens: 8000
    max_cost_usd: 0.01
  data_classifications: ["public"]
  jurisdiction: ["US"]
  network:
    egress_allowed: false
evaluation:
  critic_role: critic
  rubric:
    dimensions:
      - {name: quality, weight: 1.0, threshold: 0.5}
runtime:
  local_dev: true
completion:
  rules: ["done"]
  on_failure: abort
  max_retries: 0
provenance:
  signatures:
    - signer_did: "did:web:example.com"
      algorithm: ed25519
      signature: "AAA"
"#;

    fn fields(controls: &[UnenforcedControl]) -> Vec<&'static str> {
        controls.iter().map(|c| c.field).collect()
    }

    #[test]
    fn bare_kit_reports_only_the_required_blocks() {
        let m = parse_yaml(BARE_KIT).unwrap();
        assert_eq!(
            fields(&unenforced_on_agent_path(&m)),
            [
                "constraints.global_budget",
                "constraints.network",
                "completion"
            ]
        );
    }

    /// Regression: `kit validate` reported a kit as valid without saying
    /// that its isolation, budget, network and signature declarations do
    /// nothing on the agent path.
    #[test]
    fn every_declared_control_is_reported_once_in_order() {
        let m = parse_yaml(FULL_KIT).unwrap();
        assert_eq!(
            fields(&unenforced_on_agent_path(&m)),
            [
                "roles[].agent_provisioning.isolation",
                "roles[].agent_provisioning.budget",
                "roles[].agent_provisioning.tool_use",
                "roles[].mcp_tools",
                "roles[].context_scope",
                "roles[].tdf_attribute_release_policy",
                "roles[].handoffs",
                "roles[].skills[].signature",
                "constraints.global_budget",
                "constraints.network",
                "constraints.data_classifications",
                "constraints.jurisdiction",
                "evaluation",
                "completion",
                "provenance.signatures",
                "runtime.local_dev",
            ]
        );
    }

    #[test]
    fn role_level_controls_name_only_the_roles_that_declare_them() {
        let m = parse_yaml(FULL_KIT).unwrap();
        let controls = unenforced_on_agent_path(&m);
        let roles_for = |field: &str| {
            controls
                .iter()
                .find(|c| c.field == field)
                .map(|c| c.roles.clone())
                .unwrap()
        };
        assert_eq!(
            roles_for("roles[].agent_provisioning.budget"),
            ["planner", "critic"]
        );
        assert_eq!(
            roles_for("roles[].agent_provisioning.isolation"),
            ["planner"]
        );
        assert!(roles_for("constraints.network").is_empty());
    }

    #[test]
    fn local_dev_is_reported_for_either_value() {
        let mut m = parse_yaml(FULL_KIT).unwrap();
        m.runtime.as_mut().unwrap().local_dev = Some(false);
        assert!(fields(&unenforced_on_agent_path(&m)).contains(&"runtime.local_dev"));
        m.runtime.as_mut().unwrap().local_dev = None;
        assert!(!fields(&unenforced_on_agent_path(&m)).contains(&"runtime.local_dev"));
    }

    #[test]
    fn a_skill_with_only_signed_by_counts_as_declaring_a_signature() {
        let mut m = parse_yaml(FULL_KIT).unwrap();
        m.roles[0].skills[0].signature = None;
        assert!(fields(&unenforced_on_agent_path(&m)).contains(&"roles[].skills[].signature"));
        m.roles[0].skills[0].signed_by = None;
        assert!(!fields(&unenforced_on_agent_path(&m)).contains(&"roles[].skills[].signature"));
    }
}
