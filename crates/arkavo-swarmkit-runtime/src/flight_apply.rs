//! Atomic apply of flight-radius tightening proposals across every role ARP
//! in a flight (#609).
//!
//! Design decision (recorded): **check-then-apply with trace-emitting revert
//! on partial failure**, not two-phase commit. The flight is an in-process
//! orchestrator — every role queue is reachable under a lock — so a
//! side-effect-free admission check against all roles first removes the
//! common divergence case outright, and the proposal queue's revert path
//! (already auditable, never erased) handles the residual one. 2PC machinery
//! would serve an A2A distribution layer that is still aspirational.
//!
//! Mesh-radius proposals are never applied here: they are per-agent,
//! human-reviewed, always.

use arkavo_arp::proposal::{BlastRadius, ProposalState, TighteningProposal};

use crate::flight::SwarmFlight;

/// Why a flight-wide apply did not (fully) happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlightApplyError {
    /// Only flight-radius proposals ride this path.
    WrongRadius(BlastRadius),
    /// A role failed the admission check; nothing was applied anywhere.
    PrecheckFailed { role_id: String, reason: String },
    /// A role failed during apply after others had applied; the applied
    /// roles were reverted (each revert emitted its trace entry).
    PartialFailureReverted { role_id: String, reason: String },
}

impl std::fmt::Display for FlightApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongRadius(r) => write!(
                f,
                "flight apply accepts only flight-radius proposals, got {r:?} \
                 (mesh is never auto-applied; narrower radii use per-role ingest)"
            ),
            Self::PrecheckFailed { role_id, reason } => {
                write!(f, "admission check failed on role {role_id}: {reason}")
            }
            Self::PartialFailureReverted { role_id, reason } => write!(
                f,
                "apply failed on role {role_id} ({reason}); previously applied roles reverted"
            ),
        }
    }
}

impl std::error::Error for FlightApplyError {}

/// Apply a kit-approved flight-radius proposal to every role ARP in the
/// flight, atomically from the caller's perspective: on return it is either
/// applied everywhere (observed) or applied nowhere (with any partial
/// application reverted and trace-logged).
pub async fn apply_flight_proposal(
    flight: &SwarmFlight,
    proposal: &TighteningProposal,
    reviewer: &str,
    now_epoch_sec: u64,
) -> Result<(), FlightApplyError> {
    if proposal.blast_radius != BlastRadius::Flight {
        return Err(FlightApplyError::WrongRadius(proposal.blast_radius));
    }

    // Phase 1: side-effect-free admission check on every role. A single
    // refusal blocks the whole flight before anything mutates.
    for role in flight.roles() {
        let queue = role.arp().proposal_queue();
        let guard = queue.lock().await;
        if let Err(reason) = guard.check(proposal) {
            return Err(FlightApplyError::PrecheckFailed {
                role_id: role.role_id().to_string(),
                reason,
            });
        }
    }

    // Phase 2: apply role by role. On failure, revert the roles already
    // applied so the flight converges back to the pre-proposal state.
    let mut applied: Vec<String> = Vec::new();
    for role in flight.roles() {
        let queue = role.arp().proposal_queue();
        let mut guard = queue.lock().await;
        match guard.ingest_reviewed(proposal.clone(), reviewer, now_epoch_sec) {
            Ok(_) => applied.push(role.role_id().to_string()),
            Err(reason) => {
                drop(guard);
                revert_from_roles(flight, &applied, &proposal.id, &reason).await;
                return Err(FlightApplyError::PartialFailureReverted {
                    role_id: role.role_id().to_string(),
                    reason,
                });
            }
        }
    }
    Ok(())
}

/// Reconcile a flight after a crash mid-apply: if any role lacks the
/// proposal in an applied state, revert it from the roles that have it.
/// Returns the role ids that were reverted. Idempotent — a converged flight
/// reconciles to no-ops.
pub async fn reconcile_flight_proposal(flight: &SwarmFlight, proposal_id: &str) -> Vec<String> {
    let mut have: Vec<String> = Vec::new();
    let mut missing = false;
    for role in flight.roles() {
        let queue = role.arp().proposal_queue();
        let guard = queue.lock().await;
        match guard.proposal_state(proposal_id) {
            Some(ProposalState::Applied | ProposalState::Observed) => {
                have.push(role.role_id().to_string());
            }
            Some(ProposalState::Confirmed) => {
                // Confirmed proposals survived their observation window;
                // treat as present.
                have.push(role.role_id().to_string());
            }
            _ => missing = true,
        }
    }
    if !missing || have.is_empty() {
        return Vec::new();
    }
    revert_from_roles(
        flight,
        &have,
        proposal_id,
        "flight apply did not reach every role; reconciling to pre-proposal state",
    )
    .await;
    have
}

async fn revert_from_roles(flight: &SwarmFlight, role_ids: &[String], id: &str, reason: &str) {
    for role_id in role_ids {
        if let Some(role) = flight.role(role_id) {
            let queue = role.arp().proposal_queue();
            let mut guard = queue.lock().await;
            if let Err(e) = guard.force_revert(id, reason) {
                // A failed revert is loud, never silent: it means manual
                // attention is required on this role.
                tracing::error!(role_id = %role_id, proposal_id = %id, error = %e,
                    "flight revert failed on role");
            }
        }
    }
}
