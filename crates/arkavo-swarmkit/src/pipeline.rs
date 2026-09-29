//! The order a pipeline kit's roles run in, derived from `handoffs`.
//!
//! A kit with `coordination.topology: pipeline` describes a line of roles:
//! each hands its work to at most one other role and receives work from at
//! most one. The first role of a line is its entry role, and it is the one
//! that drives a run. Everything here is a pure function of the manifest, so
//! the same kit always yields the same plan and a kit that cannot be planned
//! is rejected when it is validated, not when a run is half done.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::coordination::{OnFailure, Topology};
use crate::manifest::Manifest;
use crate::role::RoleSpec;

mod verdict;

pub use verdict::{VERDICT_FAIL_LINE, VERDICT_PASS_LINE, Verdict, parse_verdict};

/// The `context_scope` entry that stands for the role's own output.
pub const SELF_SCOPE: &str = "self";

/// Why a manifest, or a role in it, yields no pipeline plan.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PipelineError {
    #[error("coordination.topology is {found}, not pipeline")]
    NotPipeline { found: String },

    #[error("role {0:?} is not in the kit")]
    UnknownRole(String),

    #[error(
        "role {role:?} hands off to {} roles ({}); a pipeline role hands off to at most one",
        targets.len(),
        targets.join(", ")
    )]
    FanOut { role: String, targets: Vec<String> },

    #[error(
        "role {role:?} receives handoffs from {} roles ({}); a pipeline role receives from at most one",
        sources.len(),
        sources.join(", ")
    )]
    FanIn { role: String, sources: Vec<String> },

    #[error("handoffs form a cycle: {}", path.join(" -> "))]
    Cycle { path: Vec<String> },

    #[error(
        "role {role:?} is not the entry role of its pipeline; send the request to {entry:?}, which no role hands off to"
    )]
    NotEntry { role: String, entry: String },

    #[error(
        "role {0:?} has no handoffs and no role hands off to it, so it is not part of a pipeline"
    )]
    NotInPipeline(String),
}

/// One role's place in a pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineStep {
    pub role_id: String,
    /// Earlier roles whose output this role is shown, in pipeline order.
    pub reads: Vec<String>,
}

/// The role whose verdict decides whether a run succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CriticGate {
    /// Position of `evaluation.critic_role` in [`PipelinePlan::steps`].
    /// Never 0: the role before the critic is the one asked to revise.
    pub step: usize,
    /// `completion.max_retries`: revisions allowed after a failing verdict.
    pub max_retries: u32,
    /// `completion.on_failure`: what to do once the retries are used up.
    pub on_failure: OnFailure,
}

/// Everything a driver needs to run one pipeline from its entry role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelinePlan {
    /// The roles in the order they run. The first is the entry role; there
    /// are always at least two.
    pub steps: Vec<PipelineStep>,
    /// Present when `evaluation.critic_role` is a role of this pipeline other
    /// than its entry role.
    pub critic: Option<CriticGate>,
    /// `constraints.global_budget.max_wallclock_seconds`: the whole run,
    /// retries included, has to finish inside it.
    pub max_wallclock: Duration,
}

impl PipelinePlan {
    /// The role that drives the run.
    pub fn entry_role(&self) -> &str {
        &self.steps[0].role_id
    }

    /// The role whose answer is the run's result.
    pub fn final_role(&self) -> &str {
        &self.steps[self.steps.len() - 1].role_id
    }

    /// The role id at `step`, for messages about that step.
    pub fn role_at(&self, step: usize) -> &str {
        &self.steps[step].role_id
    }
}

/// Reject handoffs that do not describe lines of roles.
///
/// A kit of any other topology passes: its handoffs are not executed, so
/// their shape constrains nothing. A pipeline kit without handoffs passes
/// too; it has no pipeline to run and each role answers for itself.
pub fn check_handoffs(m: &Manifest) -> Result<(), PipelineError> {
    if m.coordination.topology != Topology::Pipeline {
        return Ok(());
    }
    lines(m).map(|_| ())
}

/// Every pipeline in the kit, each as its role ids in running order.
///
/// Empty for a kit that is not a pipeline kit or declares no handoffs.
pub fn pipelines(m: &Manifest) -> Result<Vec<Vec<String>>, PipelineError> {
    if m.coordination.topology != Topology::Pipeline {
        return Ok(Vec::new());
    }
    Ok(lines(m)?
        .into_iter()
        .map(|line| line.into_iter().map(|role| role.id.clone()).collect())
        .collect())
}

/// The plan for the pipeline that `role_id` drives.
///
/// Fails when the kit is not a pipeline kit, and when `role_id` is not the
/// entry role of a pipeline: a run started anywhere else would skip the
/// roles before it.
pub fn plan_from(m: &Manifest, role_id: &str) -> Result<PipelinePlan, PipelineError> {
    if m.coordination.topology != Topology::Pipeline {
        return Err(PipelineError::NotPipeline {
            found: topology_name(m.coordination.topology),
        });
    }
    if !m.roles.iter().any(|r| r.id == role_id) {
        return Err(PipelineError::UnknownRole(role_id.to_string()));
    }
    let line = lines(m)?
        .into_iter()
        .find(|line| line.iter().any(|role| role.id == role_id))
        .ok_or_else(|| PipelineError::NotInPipeline(role_id.to_string()))?;
    if line[0].id != role_id {
        return Err(PipelineError::NotEntry {
            role: role_id.to_string(),
            entry: line[0].id.clone(),
        });
    }

    let steps: Vec<PipelineStep> = line
        .iter()
        .enumerate()
        .map(|(index, role)| PipelineStep {
            role_id: role.id.clone(),
            reads: reads_for(role, &line[..index]),
        })
        .collect();
    let critic = m.evaluation.as_ref().and_then(|evaluation| {
        let step = line
            .iter()
            .position(|role| role.id == evaluation.critic_role)?;
        (step > 0).then_some(CriticGate {
            step,
            max_retries: m.completion.max_retries,
            on_failure: m.completion.on_failure,
        })
    });

    Ok(PipelinePlan {
        steps,
        critic,
        max_wallclock: Duration::from_secs(m.constraints.global_budget.max_wallclock_seconds),
    })
}

/// The earlier roles whose output `role` may be shown.
///
/// A declared `context_scope` is applied as written: only the earlier roles
/// it names, `self` aside. A role that declares none is shown the output of
/// the role that handed off to it, because a handoff that hands nothing over
/// would leave the role working from the original request alone.
fn reads_for(role: &RoleSpec, earlier: &[&RoleSpec]) -> Vec<String> {
    match &role.context_scope {
        Some(scope) => earlier
            .iter()
            .map(|earlier| &earlier.id)
            .filter(|id| id.as_str() != SELF_SCOPE && scope.can_read.contains(id))
            .cloned()
            .collect(),
        None => earlier
            .last()
            .map(|before| before.id.clone())
            .into_iter()
            .collect(),
    }
}

fn topology_name(topology: Topology) -> String {
    serde_json::to_value(topology)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{topology:?}"))
}

/// Split the handoff graph into lines, or say why it is not made of lines.
///
/// Roles are visited in manifest order so the first fault reported, and the
/// order of the lines, do not depend on hashing.
fn lines(m: &Manifest) -> Result<Vec<Vec<&RoleSpec>>, PipelineError> {
    let by_id: HashMap<&str, &RoleSpec> = m.roles.iter().map(|r| (r.id.as_str(), r)).collect();
    let mut next: HashMap<&str, &str> = HashMap::new();
    let mut sources: HashMap<&str, Vec<&str>> = HashMap::new();
    for role in &m.roles {
        let mut targets: Vec<&str> = Vec::new();
        for handoff in &role.handoffs {
            if !targets.contains(&handoff.to.as_str()) {
                targets.push(&handoff.to);
            }
        }
        match targets.as_slice() {
            [] => {}
            [only] => {
                next.insert(&role.id, only);
                sources.entry(only).or_default().push(&role.id);
            }
            many => {
                return Err(PipelineError::FanOut {
                    role: role.id.clone(),
                    targets: many.iter().map(|t| (*t).to_string()).collect(),
                });
            }
        }
    }
    for role in &m.roles {
        if let Some(from) = sources.get(role.id.as_str())
            && from.len() > 1
        {
            return Err(PipelineError::FanIn {
                role: role.id.clone(),
                sources: from.iter().map(|s| (*s).to_string()).collect(),
            });
        }
    }

    let mut placed: HashSet<&str> = HashSet::new();
    let mut found = Vec::new();
    for role in &m.roles {
        let id = role.id.as_str();
        if sources.contains_key(id) || !next.contains_key(id) {
            continue;
        }
        let mut line = vec![role];
        placed.insert(id);
        let mut at = id;
        // A target that names no role ends the line; `validate` reports it
        // as an unresolved handoff before any plan is asked for.
        while let Some(&to) = next.get(at)
            && let Some(&target) = by_id.get(to)
        {
            line.push(target);
            placed.insert(to);
            at = to;
        }
        found.push(line);
    }

    // With at most one handoff in and one out per role, a role that hands
    // off and was not reached from an entry role can only sit on a cycle.
    for role in &m.roles {
        let id = role.id.as_str();
        if next.contains_key(id) && !placed.contains(id) {
            let mut path = vec![id.to_string()];
            let mut at = id;
            while let Some(&to) = next.get(at) {
                path.push(to.to_string());
                if to == id {
                    break;
                }
                at = to;
            }
            return Err(PipelineError::Cycle { path });
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests;
