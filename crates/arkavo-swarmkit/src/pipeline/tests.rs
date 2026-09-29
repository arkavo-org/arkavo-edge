use super::*;
use crate::coordination::{
    CompletionSpec, ConstraintsSpec, CoordinationSpec, EvaluationDimension, EvaluationRubric,
    EvaluationSpec, GlobalBudget, NetworkConstraints, ProvenanceSpec, Routing, RoutingStrategy,
};
use crate::manifest::{Author, KitMetadata, Objective};
use crate::role::{AgentProvisioning, ContextScope, Handoff};

/// A role as the tests describe one: its id, where it hands off, and the
/// `can_read` it declares (`None` for a role with no `context_scope`).
struct Role {
    id: &'static str,
    to: &'static [&'static str],
    can_read: Option<&'static [&'static str]>,
}

const fn role(id: &'static str, to: &'static [&'static str]) -> Role {
    Role {
        id,
        to,
        can_read: None,
    }
}

const fn scoped(
    id: &'static str,
    to: &'static [&'static str],
    can_read: &'static [&'static str],
) -> Role {
    Role {
        id,
        to,
        can_read: Some(can_read),
    }
}

fn kit(topology: Topology, roles: &[Role]) -> Manifest {
    Manifest {
        spec_version: "1.0.0".into(),
        kit: KitMetadata {
            id: String::new(),
            name: "fixture".into(),
            version: "0.1.0".into(),
            description: None,
            authors: vec![Author {
                did: "did:web:example.com".into(),
                name: None,
            }],
            created: "2026-04-29T00:00:00Z".into(),
            expires: None,
            nonce: "thz1Cz8aWOUURbyQQfvA0Q".into(),
        },
        objective: Objective {
            goal: "ship the campaign".into(),
            success_criteria: vec![],
        },
        inputs: vec![],
        deliverables: vec![],
        roles: roles
            .iter()
            .map(|r| RoleSpec {
                id: r.id.into(),
                role_type: "specialist".into(),
                plane: None,
                description: None,
                agent_provisioning: AgentProvisioning::default(),
                skills: vec![],
                mcp_tools: vec![],
                tdf_attribute_release_policy: None,
                handoffs: r
                    .to
                    .iter()
                    .map(|to| Handoff {
                        to: (*to).into(),
                        on: "always".into(),
                    })
                    .collect(),
                context_scope: r.can_read.map(|can_read| ContextScope {
                    can_read: can_read.iter().map(|s| (*s).into()).collect(),
                    can_write: vec![SELF_SCOPE.into()],
                }),
            })
            .collect(),
        coordination: CoordinationSpec {
            topology,
            protocol: "a2a-jsonrpc-2.0".into(),
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
                max_cost_usd: 1.0,
            },
            data_classifications: vec![],
            jurisdiction: vec![],
            network: NetworkConstraints {
                egress_allowed: false,
                egress_allowlist: vec![],
            },
        },
        pricing: vec![],
        evaluation: None,
        proposal_governance: None,
        runtime: None,
        completion: CompletionSpec {
            rules: vec!["done".into()],
            on_failure: OnFailure::Abort,
            max_retries: 0,
        },
        provenance: ProvenanceSpec {
            c2pa_assertions: vec![],
            signatures: vec![],
        },
    }
}

fn pipeline(roles: &[Role]) -> Manifest {
    kit(Topology::Pipeline, roles)
}

fn judged_by(mut m: Manifest, critic: &str, max_retries: u32, on_failure: OnFailure) -> Manifest {
    m.evaluation = Some(EvaluationSpec {
        rubric: EvaluationRubric {
            reference: None,
            dimensions: vec![EvaluationDimension {
                name: "quality".into(),
                weight: 1.0,
                threshold: 0.5,
            }],
        },
        critic_role: critic.into(),
        sample_size: None,
    });
    m.completion.max_retries = max_retries;
    m.completion.on_failure = on_failure;
    m
}

fn campaign() -> Manifest {
    pipeline(&[
        scoped("analyst", &["copy"], &["self"]),
        scoped("copy", &["editor"], &["analyst", "self"]),
        scoped("editor", &["critic"], &["copy", "self"]),
        scoped("critic", &[], &["analyst", "copy", "editor", "self"]),
    ])
}

fn order(plan: &PipelinePlan) -> Vec<&str> {
    plan.steps.iter().map(|s| s.role_id.as_str()).collect()
}

fn reads(plan: &PipelinePlan, role: &str) -> Vec<String> {
    plan.steps
        .iter()
        .find(|s| s.role_id == role)
        .map(|s| s.reads.clone())
        .unwrap_or_else(|| panic!("{role} is a step"))
}

#[test]
fn roles_run_in_handoff_order() {
    let plan = plan_from(&campaign(), "analyst").unwrap();
    assert_eq!(order(&plan), ["analyst", "copy", "editor", "critic"]);
    assert_eq!(plan.entry_role(), "analyst");
    assert_eq!(plan.final_role(), "critic");
    assert_eq!(plan.role_at(2), "editor");
}

/// The order comes from the handoffs, not from where a role sits in `roles`.
#[test]
fn manifest_order_does_not_decide_the_running_order() {
    let m = pipeline(&[
        role("critic", &[]),
        role("copy", &["critic"]),
        role("analyst", &["copy"]),
    ]);
    let plan = plan_from(&m, "analyst").unwrap();
    assert_eq!(order(&plan), ["analyst", "copy", "critic"]);
}

#[test]
fn a_role_reads_exactly_the_earlier_roles_it_names() {
    let plan = plan_from(&campaign(), "analyst").unwrap();
    assert!(reads(&plan, "analyst").is_empty());
    assert_eq!(reads(&plan, "copy"), ["analyst"]);
    assert_eq!(reads(&plan, "editor"), ["copy"], "analyst is not named");
    assert_eq!(reads(&plan, "critic"), ["analyst", "copy", "editor"]);
}

#[test]
fn reads_are_listed_in_pipeline_order_whatever_order_can_read_uses() {
    let m = pipeline(&[
        role("analyst", &["copy"]),
        role("copy", &["critic"]),
        scoped("critic", &[], &["self", "copy", "analyst"]),
    ]);
    let plan = plan_from(&m, "analyst").unwrap();
    assert_eq!(reads(&plan, "critic"), ["analyst", "copy"]);
}

/// `can_read` cannot reach forward, outside the pipeline, or name things
/// that are not roles: there is no output to show for any of them.
#[test]
fn names_that_are_not_earlier_roles_grant_nothing() {
    let m = pipeline(&[
        scoped("analyst", &["copy"], &["copy", "critic", "self"]),
        scoped("copy", &["critic"], &["critic", "bystander", "plan"]),
        role("critic", &[]),
        role("bystander", &[]),
    ]);
    let plan = plan_from(&m, "analyst").unwrap();
    assert!(reads(&plan, "analyst").is_empty());
    assert!(reads(&plan, "copy").is_empty());
}

#[test]
fn a_declared_scope_that_names_nobody_reads_nothing() {
    let m = pipeline(&[role("analyst", &["copy"]), scoped("copy", &[], &["self"])]);
    let plan = plan_from(&m, "analyst").unwrap();
    assert!(reads(&plan, "copy").is_empty());

    let m = pipeline(&[role("analyst", &["copy"]), scoped("copy", &[], &[])]);
    let plan = plan_from(&m, "analyst").unwrap();
    assert!(reads(&plan, "copy").is_empty());
}

#[test]
fn a_role_without_a_scope_reads_the_role_that_handed_off_to_it() {
    let m = pipeline(&[
        role("analyst", &["copy"]),
        role("copy", &["critic"]),
        role("critic", &[]),
    ]);
    let plan = plan_from(&m, "analyst").unwrap();
    assert!(reads(&plan, "analyst").is_empty());
    assert_eq!(reads(&plan, "copy"), ["analyst"]);
    assert_eq!(reads(&plan, "critic"), ["copy"]);
}

#[test]
fn the_run_budget_is_the_kit_wallclock_budget() {
    let mut m = campaign();
    m.constraints.global_budget.max_wallclock_seconds = 480;
    let plan = plan_from(&m, "analyst").unwrap();
    assert_eq!(plan.max_wallclock, Duration::from_secs(480));
}

#[test]
fn the_critic_gate_carries_the_completion_rules() {
    let m = judged_by(campaign(), "critic", 2, OnFailure::Escalate);
    let plan = plan_from(&m, "analyst").unwrap();
    assert_eq!(
        plan.critic,
        Some(CriticGate {
            step: 3,
            max_retries: 2,
            on_failure: OnFailure::Escalate,
        })
    );
}

#[test]
fn a_critic_in_the_middle_gates_the_role_before_it() {
    let m = judged_by(
        pipeline(&[
            role("analyst", &["critic"]),
            role("critic", &["publisher"]),
            role("publisher", &[]),
        ]),
        "critic",
        1,
        OnFailure::Abort,
    );
    let plan = plan_from(&m, "analyst").unwrap();
    assert_eq!(plan.critic.map(|c| c.step), Some(1));
}

#[test]
fn a_kit_without_evaluation_has_no_gate() {
    let plan = plan_from(&campaign(), "analyst").unwrap();
    assert_eq!(plan.critic, None);
}

/// The entry role has no earlier role to send back for a revision, so a
/// critic there cannot gate anything.
#[test]
fn a_critic_that_is_the_entry_role_is_not_a_gate() {
    let m = judged_by(campaign(), "analyst", 1, OnFailure::Abort);
    let plan = plan_from(&m, "analyst").unwrap();
    assert_eq!(plan.critic, None);
}

#[test]
fn a_critic_outside_the_pipeline_is_not_a_gate() {
    let m = judged_by(
        pipeline(&[
            role("analyst", &["copy"]),
            role("copy", &[]),
            role("critic", &[]),
        ]),
        "critic",
        1,
        OnFailure::Abort,
    );
    let plan = plan_from(&m, "analyst").unwrap();
    assert_eq!(order(&plan), ["analyst", "copy"]);
    assert_eq!(plan.critic, None);
}

#[test]
fn only_the_entry_role_drives() {
    for not_entry in ["copy", "editor", "critic"] {
        assert_eq!(
            plan_from(&campaign(), not_entry),
            Err(PipelineError::NotEntry {
                role: not_entry.into(),
                entry: "analyst".into(),
            })
        );
    }
}

#[test]
fn the_refusal_names_the_role_to_send_to() {
    let err = plan_from(&campaign(), "copy").unwrap_err().to_string();
    assert_eq!(
        err,
        "role \"copy\" is not the entry role of its pipeline; send the request to \
         \"analyst\", which no role hands off to"
    );
}

#[test]
fn a_role_outside_every_pipeline_does_not_drive() {
    let m = pipeline(&[
        role("analyst", &["copy"]),
        role("copy", &[]),
        role("bystander", &[]),
    ]);
    assert_eq!(
        plan_from(&m, "bystander"),
        Err(PipelineError::NotInPipeline("bystander".into()))
    );
}

#[test]
fn a_pipeline_kit_without_handoffs_has_nothing_to_drive() {
    let m = pipeline(&[role("alpha", &[]), role("beta", &[])]);
    assert_eq!(check_handoffs(&m), Ok(()));
    assert_eq!(pipelines(&m), Ok(vec![]));
    assert_eq!(
        plan_from(&m, "alpha"),
        Err(PipelineError::NotInPipeline("alpha".into()))
    );
}

#[test]
fn an_unknown_role_is_named() {
    assert_eq!(
        plan_from(&campaign(), "analist"),
        Err(PipelineError::UnknownRole("analist".into()))
    );
}

#[test]
fn other_topologies_are_not_planned() {
    for (topology, name) in [
        (Topology::Mesh, "mesh"),
        (Topology::HubSpoke, "hub-spoke"),
        (Topology::Hierarchical, "hierarchical"),
    ] {
        let m = kit(topology, &[role("analyst", &["copy"]), role("copy", &[])]);
        assert_eq!(
            plan_from(&m, "analyst"),
            Err(PipelineError::NotPipeline { found: name.into() })
        );
        assert_eq!(pipelines(&m), Ok(vec![]));
    }
}

/// Handoffs outside a pipeline kit are not executed, so their shape is not
/// this module's business.
#[test]
fn other_topologies_may_fan_out_and_loop() {
    let m = kit(
        Topology::HubSpoke,
        &[
            role("hub", &["left", "right"]),
            role("left", &["hub"]),
            role("right", &["hub"]),
        ],
    );
    assert_eq!(check_handoffs(&m), Ok(()));
}

#[test]
fn fan_out_is_rejected_naming_the_role_and_its_targets() {
    let m = pipeline(&[
        role("analyst", &["copy", "legal"]),
        role("copy", &[]),
        role("legal", &[]),
    ]);
    let expected = PipelineError::FanOut {
        role: "analyst".into(),
        targets: vec!["copy".into(), "legal".into()],
    };
    assert_eq!(check_handoffs(&m), Err(expected.clone()));
    assert_eq!(plan_from(&m, "analyst"), Err(expected.clone()));
    assert_eq!(
        expected.to_string(),
        "role \"analyst\" hands off to 2 roles (copy, legal); a pipeline role hands off to \
         at most one"
    );
}

/// Two conditions for handing off to the same role are still one line.
#[test]
fn two_handoffs_to_the_same_role_are_one_handoff() {
    let m = pipeline(&[role("analyst", &["copy", "copy"]), role("copy", &[])]);
    assert_eq!(check_handoffs(&m), Ok(()));
    assert_eq!(
        order(&plan_from(&m, "analyst").unwrap()),
        ["analyst", "copy"]
    );
}

#[test]
fn fan_in_is_rejected_naming_the_role_and_its_sources() {
    let m = pipeline(&[
        role("analyst", &["critic"]),
        role("copy", &["critic"]),
        role("critic", &[]),
    ]);
    let expected = PipelineError::FanIn {
        role: "critic".into(),
        sources: vec!["analyst".into(), "copy".into()],
    };
    assert_eq!(check_handoffs(&m), Err(expected.clone()));
    assert_eq!(
        expected.to_string(),
        "role \"critic\" receives handoffs from 2 roles (analyst, copy); a pipeline role \
         receives from at most one"
    );
}

#[test]
fn a_cycle_is_rejected_with_its_path() {
    let m = pipeline(&[
        role("analyst", &["copy"]),
        role("copy", &["critic"]),
        role("critic", &["analyst"]),
    ]);
    let expected = PipelineError::Cycle {
        path: vec![
            "analyst".into(),
            "copy".into(),
            "critic".into(),
            "analyst".into(),
        ],
    };
    assert_eq!(check_handoffs(&m), Err(expected.clone()));
    assert_eq!(plan_from(&m, "analyst"), Err(expected.clone()));
    assert_eq!(
        expected.to_string(),
        "handoffs form a cycle: analyst -> copy -> critic -> analyst"
    );
}

#[test]
fn a_role_that_hands_off_to_itself_is_a_cycle() {
    let m = pipeline(&[role("analyst", &["analyst"])]);
    assert_eq!(
        check_handoffs(&m),
        Err(PipelineError::Cycle {
            path: vec!["analyst".into(), "analyst".into()],
        })
    );
}

/// A cycle elsewhere in the kit makes the kit invalid even for a role whose
/// own line is sound: the kit is validated as a whole.
#[test]
fn a_cycle_beside_a_sound_line_is_still_rejected() {
    let m = pipeline(&[
        role("analyst", &["copy"]),
        role("copy", &[]),
        role("ping", &["pong"]),
        role("pong", &["ping"]),
    ]);
    assert_eq!(
        plan_from(&m, "analyst"),
        Err(PipelineError::Cycle {
            path: vec!["ping".into(), "pong".into(), "ping".into()],
        })
    );
}

#[test]
fn separate_lines_each_have_their_own_entry_role() {
    let m = pipeline(&[
        role("analyst", &["copy"]),
        role("copy", &[]),
        role("scout", &["mapper"]),
        role("mapper", &[]),
    ]);
    assert_eq!(
        pipelines(&m),
        Ok(vec![
            vec!["analyst".to_string(), "copy".to_string()],
            vec!["scout".to_string(), "mapper".to_string()],
        ])
    );
    assert_eq!(order(&plan_from(&m, "scout").unwrap()), ["scout", "mapper"]);
    assert_eq!(
        plan_from(&m, "mapper"),
        Err(PipelineError::NotEntry {
            role: "mapper".into(),
            entry: "scout".into(),
        })
    );
}

#[test]
fn planning_is_deterministic() {
    let m = judged_by(campaign(), "critic", 1, OnFailure::Abort);
    let first = plan_from(&m, "analyst").unwrap();
    for _ in 0..20 {
        assert_eq!(plan_from(&m, "analyst").unwrap(), first);
    }
}
