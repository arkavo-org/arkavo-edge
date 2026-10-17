//! Choosing an agent for each role and handing it the role (SwarmKit
//! draft-02 §7.1.2), apart from how agents are reached.

use std::collections::HashSet;

use arkavo_crypto::AgentKeypair;
use arkavo_swarmkit::{Manifest, RoleSpec};
use arkavo_swarmkit_delegation::{BindingMode, Delegation, Ready, Refusal, SpecialistStatus};
use chrono::{DateTime, TimeDelta, Utc};
use uuid::Uuid;

mod undo;

pub(super) use undo::{RolledBack, Undo};

/// An agent that answered `swarmkit/status`.
#[derive(Debug, Clone)]
pub(super) struct Candidate {
    pub(super) name: String,
    pub(super) address: String,
    pub(super) status: SpecialistStatus,
}

/// Why a delegation did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Failure {
    Refused(Refusal),
    /// The agent could not be reached, or answered something else.
    Unreachable(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(refusal) => write!(f, "refused: {refusal}"),
            Self::Unreachable(reason) => f.write_str(reason),
        }
    }
}

/// How the orchestrator reaches agents.
pub(super) trait Specialists: Sync {
    fn delegate(
        &self,
        address: &str,
        envelope: serde_json::Value,
    ) -> impl Future<Output = Result<Ready, Failure>> + Send;
    fn revoke(
        &self,
        address: &str,
        revocation: serde_json::Value,
    ) -> impl Future<Output = Result<(), Failure>> + Send;
}

/// A role handed to an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Assignment {
    pub(super) role: String,
    pub(super) agent: String,
    pub(super) specialist: String,
    pub(super) expires: DateTime<Utc>,
}

/// Every role handed out, and the agents that may have taken a role this
/// apply could not confirm, put back as they were.
#[derive(Debug)]
pub(super) struct Applied {
    pub(super) flight: Uuid,
    pub(super) assignments: Vec<Assignment>,
    pub(super) cleaned_up: Vec<RolledBack>,
}

/// A role no agent took, with what each one it was offered to said, and
/// every agent this apply touched put back as it was.
#[derive(Debug)]
pub(super) struct Unassigned {
    pub(super) role: String,
    pub(super) offers: Vec<(String, Failure)>,
    pub(super) rolled_back: Vec<RolledBack>,
}

/// Whether the orchestrator `me` may offer a role to `status`'s agent, or
/// why not (§7.1.2 step 1).
pub(super) fn eligibility(status: &SpecialistStatus, me: &str) -> Result<(), String> {
    match &status.binding {
        Some(binding) if binding.orchestrator == me => Ok(()),
        Some(binding) => Err(format!("bound to {}", binding.orchestrator)),
        None if status.binds_by.contains(&BindingMode::FirstUse) => Ok(()),
        None => Err("takes no role from an orchestrator".into()),
    }
}

/// Roles that need host capabilities first, so the hosts that have them go
/// to those roles (§7.1.2 step 2).
pub(super) fn role_order(roles: &[RoleSpec]) -> Vec<&RoleSpec> {
    let mut ordered: Vec<&RoleSpec> = roles.iter().collect();
    ordered.sort_by_key(|role| role.required_capabilities().is_empty());
    ordered
}

/// Candidates for a role that needs `needs`, best first: every capability
/// present, ready before not, then the least loaded, then by name.
pub(super) fn ranked<'a>(
    needs: &[String],
    candidates: &'a [Candidate],
    taken: &HashSet<String>,
) -> Vec<&'a Candidate> {
    let mut fit: Vec<(&Candidate, bool)> = candidates
        .iter()
        .filter(|c| !taken.contains(&c.status.specialist))
        .filter_map(|c| {
            let ready: Option<Vec<bool>> = needs.iter().map(|n| c.status.capability(n)).collect();
            ready.map(|ready| (c, ready.iter().all(|r| *r)))
        })
        .collect();
    fit.sort_by(|(a, a_ready), (b, b_ready)| {
        b_ready
            .cmp(a_ready)
            .then(a.status.load.total_cmp(&b.status.load))
            .then(a.name.cmp(&b.name))
    });
    fit.into_iter().map(|(c, _)| c).collect()
}

/// Hand each role of `manifest` to one candidate, all or none: a role no
/// candidate takes puts every agent this apply touched back as it was.
pub(super) async fn assign<S: Specialists>(
    manifest: &Manifest,
    keypair: &AgentKeypair,
    candidates: &[Candidate],
    lifetime: TimeDelta,
    now: DateTime<Utc>,
    net: &S,
) -> Result<Applied, Unassigned> {
    let me = keypair.public_key().to_did_key();
    let flight = Uuid::new_v4();
    let mut taken = HashSet::new();
    let mut assigned: Vec<(Assignment, &Candidate)> = Vec::new();
    // Offers whose outcome is unknown: the agent may have taken the role.
    let mut unconfirmed: Vec<(String, &Candidate)> = Vec::new();
    for role in role_order(&manifest.roles) {
        let mut offers = Vec::new();
        for candidate in ranked(&role.required_capabilities(), candidates, &taken) {
            let key = &candidate.status.specialist;
            let signed = match Delegation::for_role(
                manifest,
                role,
                me.clone(),
                key.clone(),
                flight,
                now,
                lifetime,
            )
            .and_then(|d| d.sign(keypair))
            {
                Ok(signed) => signed,
                // The kit itself is the problem; every candidate would fail.
                Err(e) => {
                    offers.push((candidate.name.clone(), Failure::Unreachable(e.to_string())));
                    break;
                }
            };
            match net
                .delegate(&candidate.address, signed.wire().clone())
                .await
            {
                Ok(ready)
                    if ready.envelope_blake3 == signed.envelope_blake3()
                        && &ready.specialist == key =>
                {
                    taken.insert(key.clone());
                    let assignment = Assignment {
                        role: role.id.clone(),
                        agent: candidate.name.clone(),
                        specialist: key.clone(),
                        expires: signed.delegation().expires,
                    };
                    assigned.push((assignment, candidate));
                    break;
                }
                Ok(_) => {
                    unconfirmed.push((role.id.clone(), candidate));
                    offers.push((
                        candidate.name.clone(),
                        Failure::Unreachable("acknowledged a different envelope".into()),
                    ));
                }
                Err(failure) => {
                    if !matches!(failure, Failure::Refused(_)) {
                        unconfirmed.push((role.id.clone(), candidate));
                    }
                    offers.push((candidate.name.clone(), failure));
                }
            }
        }
        if !assigned.iter().any(|(a, _)| a.role == role.id) {
            let touched = assigned
                .iter()
                .map(|(a, c)| (a.role.clone(), *c))
                .chain(unconfirmed);
            let rolled_back = undo_all(manifest, touched, keypair, flight, now, net).await;
            return Err(Unassigned {
                role: role.id.clone(),
                offers,
                rolled_back,
            });
        }
    }
    // An agent whose offer went unconfirmed and that then took no role may
    // still hold one nobody counts on.
    let stray = unconfirmed
        .into_iter()
        .filter(|(_, c)| !taken.contains(&c.status.specialist));
    let cleaned_up = undo_all(manifest, stray, keypair, flight, now, net).await;
    Ok(Applied {
        flight,
        assignments: assigned.into_iter().map(|(a, _)| a).collect(),
        cleaned_up,
    })
}

/// Undo each agent once, naming the role it was offered.
async fn undo_all<'a, S: Specialists>(
    manifest: &Manifest,
    touched: impl Iterator<Item = (String, &'a Candidate)>,
    keypair: &AgentKeypair,
    flight: Uuid,
    now: DateTime<Utc>,
    net: &S,
) -> Vec<RolledBack> {
    let mut seen = HashSet::new();
    let mut rolled_back = Vec::new();
    for (role, candidate) in touched {
        if !seen.insert(candidate.status.specialist.clone()) {
            continue;
        }
        let undo = undo::undo(manifest, candidate, keypair, flight, now, net).await;
        rolled_back.push(RolledBack {
            agent: candidate.name.clone(),
            role,
            undo,
        });
    }
    rolled_back
}

#[cfg(test)]
mod tests;
