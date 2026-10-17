//! Putting an agent back as it was before an apply touched it.
//!
//! An apply touches an agent when it hands it a role, and also when an
//! offer's outcome is unknown: a timeout or a dropped connection can come
//! after the agent committed the role. Undoing revokes this flight's role,
//! releases a binding the agent did not have before, and gives back a role
//! the agent held from an earlier flight of the same kit, which the new
//! role replaced.

use arkavo_crypto::AgentKeypair;
use arkavo_swarmkit::Manifest;
use arkavo_swarmkit_delegation::{Delegation, RefusalReason, Revocation};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::{Candidate, Failure, Specialists};

/// What undoing did to one agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::commands::kit::apply) enum Undo {
    /// The agent is as it was before the apply.
    Restored,
    /// This flight's role was taken back, but the agent's earlier role came
    /// from another kit, so this apply cannot give it back.
    EarlierRoleLost { role: String, kit_id: String },
    /// The agent refused or could not be reached, so it may still hold this
    /// flight's role.
    Failed(Failure),
}

/// One agent an apply touched, the role it was offered, and what undoing
/// did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::commands::kit::apply) struct RolledBack {
    pub(in crate::commands::kit::apply) agent: String,
    pub(in crate::commands::kit::apply) role: String,
    pub(in crate::commands::kit::apply) undo: Undo,
}

/// Put `candidate` back as its status said it was before flight `flight`.
pub(super) async fn undo<S: Specialists>(
    manifest: &Manifest,
    candidate: &Candidate,
    keypair: &AgentKeypair,
    flight: Uuid,
    now: DateTime<Utc>,
    net: &S,
) -> Undo {
    let me = keypair.public_key().to_did_key();
    let before = &candidate.status;
    let was_unbound = before.binding.is_none();
    let revocation = Revocation::new(
        me.clone(),
        before.specialist.clone(),
        Some(flight),
        was_unbound,
        Utc::now(),
    );
    let signed = match revocation.sign(keypair) {
        Ok(signed) => signed,
        Err(e) => return Undo::Failed(Failure::Unreachable(e.to_string())),
    };
    match net.revoke(&candidate.address, signed.wire().clone()).await {
        Ok(()) => {}
        // An agent that never took the role never bound to this
        // orchestrator either: it is as it was.
        Err(Failure::Refused(refusal))
            if was_unbound && refusal.reason == RefusalReason::NotBoundOrchestrator => {}
        Err(failure) => return Undo::Failed(failure),
    }
    let Some(earlier) = before
        .delegation
        .as_ref()
        .filter(|d| !was_unbound && d.flight_id != flight && d.expires > now)
    else {
        return Undo::Restored;
    };
    // An agent that never took this flight's role still holds its earlier
    // one, and the flight-scoped revocation left it alone.
    if let Ok(after) = net.status(&candidate.address).await
        && after
            .delegation
            .is_some_and(|d| d.flight_id == earlier.flight_id)
    {
        return Undo::Restored;
    }
    let lost = || Undo::EarlierRoleLost {
        role: earlier.role_id.clone(),
        kit_id: earlier.kit_id.clone(),
    };
    let Some(role) = manifest
        .roles
        .iter()
        .find(|r| r.id == earlier.role_id)
        .filter(|_| earlier.kit_id == manifest.kit.id)
    else {
        return lost();
    };
    // The earlier role goes back under its own flight and until its own
    // expiry, in a fresh envelope: the agent refuses the old one as older
    // than the role this flight gave it.
    let signed = match Delegation::for_role(
        manifest,
        role,
        me,
        before.specialist.clone(),
        earlier.flight_id,
        now,
        earlier.expires - now,
    )
    .and_then(|d| d.sign(keypair))
    {
        Ok(signed) => signed,
        Err(_) => return lost(),
    };
    match net
        .delegate(&candidate.address, signed.wire().clone())
        .await
    {
        Ok(ready) if ready.envelope_blake3 == signed.envelope_blake3() => Undo::Restored,
        _ => lost(),
    }
}
