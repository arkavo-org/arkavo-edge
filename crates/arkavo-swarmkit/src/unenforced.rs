//! Controls a manifest declares that `arkavo agent -c <kit>` does not act on.
//!
//! The CLI agent path builds one agent per role from the role's id, model
//! and skill instructions, plus the kit's `runtime` block. A kit whose
//! topology is `pipeline` also has its handoffs run: a message to the entry
//! role runs the roles in order, each shown what its `context_scope` lets it
//! read, under the kit's wallclock budget and the critic's verdict. The rest
//! of the manifest parses and validates, which can read as if it were in
//! force. This module names the declared fields that have no consumer on
//! that path so a tool can tell the author before they rely on them.
//!
//! The list is scoped to that one entry point. Some of these fields are read
//! elsewhere (for example `SwarmFlight::launch` verifies skill signatures
//! when a resolver is configured); a field belongs here when nothing the
//! agent process starts reads it.

use std::collections::HashSet;

use crate::coordination::OnFailure;
use crate::manifest::Manifest;
use crate::pipeline::pipelines;
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

/// What running the kit as a pipeline puts in force.
struct PipelineRun<'a> {
    /// Roles a pipeline run reaches: their handoffs are followed and their
    /// `can_read` decides what they are shown.
    roles: HashSet<&'a str>,
    /// `evaluation.critic_role` reviews the work of a run, so its verdict,
    /// `completion.max_retries` and `completion.on_failure` apply.
    gated: bool,
}

impl<'a> PipelineRun<'a> {
    fn of(m: &'a Manifest) -> Self {
        // A kit that reaches this module has been validated, so its handoffs
        // can be planned. One that cannot be runs nothing as a pipeline.
        let lines = pipelines(m).unwrap_or_default();
        let gated = m.evaluation.as_ref().is_some_and(|evaluation| {
            lines.iter().any(|line| {
                line.iter()
                    .position(|id| *id == evaluation.critic_role)
                    .is_some_and(|step| step > 0)
            })
        });
        let on_a_line: HashSet<&str> = lines.iter().flatten().map(String::as_str).collect();
        Self {
            roles: m
                .roles
                .iter()
                .map(|r| r.id.as_str())
                .filter(|id| on_a_line.contains(id))
                .collect(),
            gated,
        }
    }

    fn reaches(&self, role: &RoleSpec) -> bool {
        self.roles.contains(role.id.as_str())
    }

    fn runs(&self) -> bool {
        !self.roles.is_empty()
    }
}

struct RoleRule {
    field: &'static str,
    effect: &'static str,
    declared: fn(&RoleSpec, &PipelineRun<'_>) -> bool,
}

/// A kit-level control: the field and effect to report, when the kit
/// declares something of it that is not in force.
type KitRule = fn(&Manifest, &PipelineRun<'_>) -> Option<(&'static str, &'static str)>;

const ROLE_RULES: &[RoleRule] = &[
    RoleRule {
        field: "roles[].agent_provisioning.isolation",
        effect: "sandbox, fs_writable and network_egress are not applied to the agent process",
        declared: |r, _| r.agent_provisioning.isolation.is_some(),
    },
    RoleRule {
        field: "roles[].agent_provisioning.budget",
        effect: "max_inference_calls, max_wallclock_ms and max_total_tokens are not applied as limits",
        declared: |r, _| r.agent_provisioning.budget.is_some(),
    },
    RoleRule {
        field: "roles[].agent_provisioning.tool_use",
        effect: "max_calls, max_parallel, on_error and retry_policy are not applied",
        declared: |r, _| r.agent_provisioning.tool_use.is_some(),
    },
    RoleRule {
        field: "roles[].agent_provisioning.inference",
        effect: "max_tokens, temperature, top_p, top_k, thinking, stop_sequences and seed are not passed to the model",
        declared: |r, _| r.agent_provisioning.inference.is_some(),
    },
    RoleRule {
        field: "roles[].agent_provisioning.context",
        effect: "max_context_tokens, kv_cache_id, compaction_strategy and persistence are not applied",
        declared: |r, _| r.agent_provisioning.context.is_some(),
    },
    RoleRule {
        field: "roles[].mcp_tools",
        effect: "grants are not used to restrict which tools the role can call",
        declared: |r, _| !r.mcp_tools.is_empty(),
    },
    RoleRule {
        field: "roles[].context_scope",
        effect: "can_read and can_write are not applied",
        declared: |r, run| r.context_scope.is_some() && !run.reaches(r),
    },
    RoleRule {
        field: "roles[].context_scope.can_write",
        effect: "can_write is not applied; roles share no context store to write to",
        declared: |r, run| {
            run.reaches(r)
                && r.context_scope
                    .as_ref()
                    .is_some_and(|scope| !scope.can_write.is_empty())
        },
    },
    RoleRule {
        field: "roles[].tdf_attribute_release_policy",
        effect: "no TDF policy is built from the role's attributes",
        declared: |r, _| r.tdf_attribute_release_policy.is_some(),
    },
    RoleRule {
        field: "roles[].handoffs",
        effect: "handoff targets are not used for routing",
        declared: |r, run| !r.handoffs.is_empty() && !run.reaches(r),
    },
    RoleRule {
        field: "roles[].skills[].signature",
        effect: "skill signatures are not verified; skill instructions are used as written",
        declared: |r, _| {
            r.skills
                .iter()
                .any(|s| s.signature.is_some() || s.signed_by.is_some())
        },
    },
];

const KIT_RULES: &[KitRule] = &[
    |_, run| {
        Some(if run.runs() {
            (
                "constraints.global_budget",
                "max_total_tokens and max_cost_usd are not applied as limits; max_wallclock_seconds bounds a pipeline run and nothing else",
            )
        } else {
            (
                "constraints.global_budget",
                "max_wallclock_seconds, max_total_tokens and max_cost_usd are not applied as limits",
            )
        })
    },
    |_, _| {
        Some((
            "constraints.network",
            "egress_allowed and egress_allowlist do not restrict network access",
        ))
    },
    |m, _| {
        (!m.constraints.data_classifications.is_empty()).then_some((
            "constraints.data_classifications",
            "classifications are not applied to data the agent handles",
        ))
    },
    |m, _| {
        (!m.constraints.jurisdiction.is_empty())
            .then_some(("constraints.jurisdiction", "jurisdictions are not applied"))
    },
    |m, run| {
        m.evaluation.as_ref().map(|_| {
            if run.gated {
                (
                    "evaluation.rubric",
                    "the rubric is not scored and sample_size is not applied; the critic's verdict line alone decides",
                )
            } else {
                (
                    "evaluation",
                    "the rubric is not scored and critic_role is not consulted",
                )
            }
        })
    },
    |_, run| {
        Some(if run.gated {
            ("completion.rules", "rules are not evaluated")
        } else {
            (
                "completion",
                "rules, on_failure and max_retries are not evaluated",
            )
        })
    },
    |m, run| {
        (run.gated && m.completion.on_failure != OnFailure::Abort).then_some((
            "completion.on_failure",
            "only abort is carried out; this value ends a failed run the way abort does",
        ))
    },
    |m, _| {
        (!m.provenance.signatures.is_empty()).then_some((
            "provenance.signatures",
            "manifest signatures are not verified",
        ))
    },
    |m, _| {
        m.runtime
            .as_ref()
            .is_some_and(|r| r.local_dev.is_some())
            .then_some((
                "runtime.local_dev",
                "has no effect; skill signatures are not verified for either value",
            ))
    },
];

/// The controls `m` declares that have no effect when the kit is run with
/// `arkavo agent -c <kit>`, role-level fields first, each in a fixed order.
///
/// A field is reported only when the manifest sets it, so the result
/// describes this kit and not the format. `constraints.global_budget`,
/// `constraints.network` and `completion` are required blocks and therefore
/// always reported, in whole or for the part of them that is not in force.
///
/// What a pipeline run puts in force is left out for the roles and kits it
/// applies to, and only for those: a role no handoff reaches is not part of
/// a run, and a kit of another topology has no run at all.
pub fn unenforced_on_agent_path(m: &Manifest) -> Vec<UnenforcedControl> {
    let run = PipelineRun::of(m);
    let role_level = ROLE_RULES.iter().filter_map(|rule| {
        let roles: Vec<String> = m
            .roles
            .iter()
            .filter(|role| (rule.declared)(role, &run))
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
        .filter_map(|rule| rule(m, &run))
        .map(|(field, effect)| UnenforcedControl {
            field,
            roles: Vec::new(),
            effect,
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

    /// A kit that declares every control, under `topology`.
    fn full_kit(topology: &str) -> Manifest {
        let yaml = FULL_KIT.replace("{topology}", topology);
        parse_yaml(&yaml).unwrap()
    }

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
      inference: {max_tokens: 512, temperature: 0.2}
      context: {max_context_tokens: 4096, persistence: ephemeral}
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
      can_read: ["self"]
      can_write: ["self"]
  - id: critic
    role_type: critic
    agent_provisioning:
      budget: {max_total_tokens: 2000}
    context_scope:
      can_read: ["planner", "self"]
  - id: bystander
    role_type: observer
    agent_provisioning: {}
    context_scope:
      can_read: ["planner"]
coordination:
  topology: {topology}
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

    fn control<'a>(controls: &'a [UnenforcedControl], field: &str) -> &'a UnenforcedControl {
        controls
            .iter()
            .find(|c| c.field == field)
            .unwrap_or_else(|| panic!("{field} is reported"))
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
        let m = full_kit("hub-spoke");
        assert_eq!(
            fields(&unenforced_on_agent_path(&m)),
            [
                "roles[].agent_provisioning.isolation",
                "roles[].agent_provisioning.budget",
                "roles[].agent_provisioning.tool_use",
                "roles[].agent_provisioning.inference",
                "roles[].agent_provisioning.context",
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

    /// Regression: `inference` and `context` parse and validate, nothing
    /// reads them, and the report did not say so.
    #[test]
    fn inference_and_context_settings_are_reported() {
        let controls = unenforced_on_agent_path(&full_kit("pipeline"));
        let inference = control(&controls, "roles[].agent_provisioning.inference");
        assert_eq!(inference.roles, ["planner"]);
        assert_eq!(
            inference.effect,
            "max_tokens, temperature, top_p, top_k, thinking, stop_sequences and seed are \
             not passed to the model"
        );
        let context = control(&controls, "roles[].agent_provisioning.context");
        assert_eq!(context.roles, ["planner"]);
        assert_eq!(
            context.effect,
            "max_context_tokens, kv_cache_id, compaction_strategy and persistence are not \
             applied"
        );
    }

    /// What a pipeline run carries out is no longer reported for the kit it
    /// is carried out for, and what it leaves alone still is.
    #[test]
    fn a_pipeline_kit_reports_only_what_its_run_leaves_alone() {
        let controls = unenforced_on_agent_path(&full_kit("pipeline"));
        assert_eq!(
            fields(&controls),
            [
                "roles[].agent_provisioning.isolation",
                "roles[].agent_provisioning.budget",
                "roles[].agent_provisioning.tool_use",
                "roles[].agent_provisioning.inference",
                "roles[].agent_provisioning.context",
                "roles[].mcp_tools",
                "roles[].context_scope",
                "roles[].context_scope.can_write",
                "roles[].tdf_attribute_release_policy",
                "roles[].skills[].signature",
                "constraints.global_budget",
                "constraints.network",
                "constraints.data_classifications",
                "constraints.jurisdiction",
                "evaluation.rubric",
                "completion.rules",
                "provenance.signatures",
                "runtime.local_dev",
            ]
        );
        assert_eq!(
            control(&controls, "constraints.global_budget").effect,
            "max_total_tokens and max_cost_usd are not applied as limits; \
             max_wallclock_seconds bounds a pipeline run and nothing else"
        );
        assert_eq!(
            control(&controls, "evaluation.rubric").effect,
            "the rubric is not scored and sample_size is not applied; the critic's verdict \
             line alone decides"
        );
        assert_eq!(
            control(&controls, "completion.rules").effect,
            "rules are not evaluated"
        );
    }

    /// `can_read` is applied to the roles a run reaches. A role no handoff
    /// reaches is never part of a run, so its whole scope is still reported;
    /// `can_write` is reported for every role that declares one.
    #[test]
    fn context_scope_is_reported_for_the_part_and_the_roles_not_in_force() {
        let controls = unenforced_on_agent_path(&full_kit("pipeline"));
        assert_eq!(
            control(&controls, "roles[].context_scope").roles,
            ["bystander"]
        );
        assert_eq!(
            control(&controls, "roles[].context_scope.can_write").roles,
            ["planner"],
            "critic declares no can_write"
        );
    }

    #[test]
    fn an_on_failure_other_than_abort_is_reported_in_a_gated_pipeline() {
        let mut m = full_kit("pipeline");
        m.completion.on_failure = OnFailure::Escalate;
        let controls = unenforced_on_agent_path(&m);
        assert_eq!(
            control(&controls, "completion.on_failure").effect,
            "only abort is carried out; this value ends a failed run the way abort does"
        );

        m.coordination.topology = crate::coordination::Topology::HubSpoke;
        let fields = fields(&unenforced_on_agent_path(&m));
        assert!(fields.contains(&"completion"), "{fields:?}");
        assert!(!fields.contains(&"completion.on_failure"), "{fields:?}");
    }

    /// A pipeline kit whose roles hand off to nobody runs nothing in order,
    /// so everything a run would have put in force is still reported.
    #[test]
    fn a_pipeline_kit_without_handoffs_enforces_nothing_more() {
        let mut m = full_kit("pipeline");
        m.roles[0].handoffs.clear();
        let controls = unenforced_on_agent_path(&m);
        let fields = fields(&controls);
        for whole in ["roles[].context_scope", "evaluation", "completion"] {
            assert!(fields.contains(&whole), "{whole} in {fields:?}");
        }
        for part in [
            "roles[].context_scope.can_write",
            "evaluation.rubric",
            "completion.rules",
        ] {
            assert!(!fields.contains(&part), "{part} in {fields:?}");
        }
        assert_eq!(
            control(&controls, "roles[].context_scope").roles,
            ["planner", "critic", "bystander"]
        );
        assert_eq!(
            control(&controls, "constraints.global_budget").effect,
            "max_wallclock_seconds, max_total_tokens and max_cost_usd are not applied as limits"
        );
    }

    /// A critic that is the entry role has no earlier role to send back, so
    /// the run has no gate and the completion rules have nothing to act on.
    #[test]
    fn a_critic_that_cannot_gate_leaves_evaluation_and_completion_reported() {
        let mut m = full_kit("pipeline");
        m.evaluation.as_mut().unwrap().critic_role = "planner".into();
        let fields_for_entry_critic = fields(&unenforced_on_agent_path(&m));
        m.evaluation.as_mut().unwrap().critic_role = "bystander".into();
        let fields_for_outside_critic = fields(&unenforced_on_agent_path(&m));

        for fields in [fields_for_entry_critic, fields_for_outside_critic] {
            assert!(fields.contains(&"evaluation"), "{fields:?}");
            assert!(fields.contains(&"completion"), "{fields:?}");
            assert!(!fields.contains(&"roles[].handoffs"), "{fields:?}");
        }
    }

    #[test]
    fn role_level_controls_name_only_the_roles_that_declare_them() {
        let m = full_kit("hub-spoke");
        let controls = unenforced_on_agent_path(&m);
        let roles_for = |field: &str| control(&controls, field).roles.clone();
        assert_eq!(
            roles_for("roles[].agent_provisioning.budget"),
            ["planner", "critic"]
        );
        assert_eq!(
            roles_for("roles[].agent_provisioning.isolation"),
            ["planner"]
        );
        assert_eq!(roles_for("roles[].handoffs"), ["planner"]);
        assert!(roles_for("constraints.network").is_empty());
    }

    #[test]
    fn local_dev_is_reported_for_either_value() {
        let mut m = full_kit("pipeline");
        m.runtime.as_mut().unwrap().local_dev = Some(false);
        assert!(fields(&unenforced_on_agent_path(&m)).contains(&"runtime.local_dev"));
        m.runtime.as_mut().unwrap().local_dev = None;
        assert!(!fields(&unenforced_on_agent_path(&m)).contains(&"runtime.local_dev"));
    }

    #[test]
    fn a_skill_with_only_signed_by_counts_as_declaring_a_signature() {
        let mut m = full_kit("pipeline");
        m.roles[0].skills[0].signature = None;
        assert!(fields(&unenforced_on_agent_path(&m)).contains(&"roles[].skills[].signature"));
        m.roles[0].skills[0].signed_by = None;
        assert!(!fields(&unenforced_on_agent_path(&m)).contains(&"roles[].skills[].signature"));
    }
}
