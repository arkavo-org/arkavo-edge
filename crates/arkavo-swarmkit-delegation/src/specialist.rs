//! The specialist's side of delegation: whether to accept an envelope or a
//! revocation, and what it holds afterwards (spec §7.3, §7.4).
//!
//! Accepting a delegation takes two calls. [`Specialist::admit`] makes the
//! checks of §7.3 steps 1 to 5 and changes nothing. The caller then checks
//! its host (step 6) and applies the role, and [`Specialist::commit`]
//! records the binding, the delegation and its nonce (step 7). A role the
//! host refuses therefore never binds the specialist.

use chrono::{DateTime, TimeDelta, Utc};
use serde_json::Value;

use crate::acceptance::{CLOCK_SKEW, NonceCache, check_envelope, well_formed_nonce};
use crate::envelope::{Delegation, SignedDelegation};
use crate::protocol::{
    Binding, BindingMode, CapabilityReport, Ready, ReadyStatus, Refusal, RefusalReason, Revoked,
    RevokedStatus, SpecialistStatus,
};
use crate::revocation::SignedRevocation;
use crate::store::{StateStore, StoreError};

/// The largest envelope a specialist reads (§7.3 step 1).
const MAX_ENVELOPE_BYTES: usize = 1 << 20;

/// How long a revocation's nonce is kept (§7.4).
const REVOCATION_NONCE_TTL: TimeDelta = TimeDelta::minutes(10);

const NONCE_CAPACITY: usize = 4096;

/// Which bindings the specialist makes by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Whether an unbound specialist binds to the first orchestrator whose
    /// delegation it accepts (§7.3.1 `first-use`).
    pub first_use: bool,
}

/// Why a revocation was not carried out.
#[derive(Debug, thiserror::Error)]
pub enum RevokeError {
    #[error(transparent)]
    Refused(#[from] Refusal),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// An envelope that passed §7.3 steps 1 to 5, not yet recorded.
#[derive(Debug)]
pub struct Admitted {
    signed: SignedDelegation,
    binds: Option<Binding>,
}

impl Admitted {
    pub fn delegation(&self) -> &Delegation {
        self.signed.delegation()
    }

    /// Whether accepting this envelope binds the specialist on first use.
    pub fn binds(&self) -> bool {
        self.binds.is_some()
    }
}

/// A specialist key's binding, active delegation and seen nonces.
#[derive(Debug)]
pub struct Specialist {
    key: String,
    store: StateStore,
    policy: Policy,
    binding: Option<Binding>,
    active: Option<SignedDelegation>,
    nonces: NonceCache,
    revocation_nonces: NonceCache,
}

impl Specialist {
    /// Load the specialist `key` from `store`. A stored delegation that no
    /// longer verifies, has expired, or came from another orchestrator than
    /// the binding is dropped.
    pub fn open(
        key: String,
        store: StateStore,
        policy: Policy,
        now: DateTime<Utc>,
    ) -> Result<Self, StoreError> {
        let binding = store.load_binding()?;
        let stored = store.load_delegation()?;
        let active = stored
            .clone()
            .and_then(|wire| SignedDelegation::from_wire(wire).ok())
            .filter(|signed| {
                let d = signed.delegation();
                d.specialist == key
                    && d.expires > now
                    && binding
                        .as_ref()
                        .is_some_and(|b| b.orchestrator == d.orchestrator)
            });
        if stored.is_some() && active.is_none() {
            store.save_delegation(None)?;
        }
        Ok(Self {
            key,
            store,
            policy,
            binding,
            active,
            nonces: NonceCache::new(NONCE_CAPACITY),
            revocation_nonces: NonceCache::new(NONCE_CAPACITY),
        })
    }

    /// This specialist's `did:key`.
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn binding(&self) -> Option<&Binding> {
        self.binding.as_ref()
    }

    pub fn active(&self) -> Option<&Delegation> {
        self.active.as_ref().map(SignedDelegation::delegation)
    }

    /// `swarmkit/status` (§7.3.4), with what the host reports about itself.
    pub fn status(&self, capabilities: Vec<CapabilityReport>, load: f64) -> SpecialistStatus {
        let binds_by = if self.binding.is_none() && self.policy.first_use {
            vec![BindingMode::FirstUse]
        } else {
            Vec::new()
        };
        SpecialistStatus {
            specialist: self.key.clone(),
            binding: self.binding.clone(),
            binds_by,
            delegation: self.active().map(Delegation::summary),
            capabilities,
            load: load.clamp(0.0, 1.0),
        }
    }

    /// Bind to `orchestrator` on the local operator's word (§7.3.1
    /// `operator`). A delegation from another orchestrator is dropped with
    /// the binding it came under.
    pub fn bind_operator(
        &mut self,
        orchestrator: &str,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        arkavo_crypto::AgentPublicKey::from_did_key(orchestrator)
            .map_err(|_| StoreError::NotADidKey(orchestrator.to_string()))?;
        if self
            .binding
            .as_ref()
            .is_some_and(|b| b.orchestrator == orchestrator && b.mode == BindingMode::Operator)
        {
            return Ok(());
        }
        let binding = Binding {
            orchestrator: orchestrator.to_string(),
            mode: BindingMode::Operator,
            since: now,
        };
        self.store.save_binding(Some(&binding))?;
        if self
            .active()
            .is_some_and(|d| d.orchestrator != orchestrator)
        {
            self.drop_active()?;
        }
        self.binding = Some(binding);
        Ok(())
    }

    /// Forget the binding and the delegation, on the local operator's word.
    pub fn release(&mut self) -> Result<(), StoreError> {
        self.drop_active()?;
        self.store.save_binding(None)?;
        self.binding = None;
        Ok(())
    }

    /// §7.3 steps 1 to 5, changing nothing.
    pub fn admit(&self, wire: Value, now: DateTime<Utc>) -> Result<Admitted, Refusal> {
        if arkavo_swarmkit::canonical_json(&wire).len() > MAX_ENVELOPE_BYTES {
            return Err(Refusal::new(
                RefusalReason::InvalidEnvelope,
                format!("larger than {MAX_ENVELOPE_BYTES} bytes"),
            ));
        }
        let signed = SignedDelegation::from_wire(wire)?;
        let delegation = signed.delegation();
        check_envelope(delegation, &self.key, &self.nonces, now)?;
        let binds = match &self.binding {
            Some(bound) if bound.orchestrator == delegation.orchestrator => None,
            Some(bound) => {
                return Err(Refusal::new(
                    RefusalReason::NotBoundOrchestrator,
                    format!("bound to {}", bound.orchestrator),
                ));
            }
            None if self.policy.first_use => Some(Binding {
                orchestrator: delegation.orchestrator.clone(),
                mode: BindingMode::FirstUse,
                since: now,
            }),
            None => {
                return Err(Refusal::new(
                    RefusalReason::BindingRequired,
                    "this agent takes no role from an orchestrator",
                ));
            }
        };
        // The nonce cache does not survive a restart, so an older envelope
        // from the same orchestrator could otherwise roll the role back.
        if let Some(active) = self.active()
            && active.orchestrator == delegation.orchestrator
            && delegation.issued < active.issued
        {
            return Err(Refusal::new(
                RefusalReason::Replayed,
                format!("issued before the active delegation ({})", active.issued),
            ));
        }
        delegation.verify_skills()?;
        Ok(Admitted { signed, binds })
    }

    /// Record an admitted delegation once the caller has applied it (§7.3
    /// step 7), replacing any it held.
    pub fn commit(&mut self, admitted: Admitted, now: DateTime<Utc>) -> Result<Ready, StoreError> {
        let Admitted { signed, binds } = admitted;
        if let Some(binding) = binds {
            self.store.save_binding(Some(&binding))?;
            self.binding = Some(binding);
        }
        self.store.save_delegation(Some(signed.wire()))?;
        let d = signed.delegation();
        self.nonces
            .record(&d.orchestrator, &d.nonce, d.expires, now);
        let ready = Ready {
            status: ReadyStatus::Ready,
            specialist: self.key.clone(),
            flight_id: d.flight_id,
            role_id: d.role_id.clone(),
            envelope_blake3: signed.envelope_blake3(),
        };
        self.active = Some(signed);
        Ok(ready)
    }

    /// `swarmkit/revoke` (§7.4). Returns the result and whether the active
    /// delegation was dropped, so the caller can take the role back.
    pub fn revoke(
        &mut self,
        wire: Value,
        now: DateTime<Utc>,
    ) -> Result<(Revoked, bool), RevokeError> {
        let signed = SignedRevocation::from_wire(wire)?;
        let r = signed.revocation();
        if r.specialist != self.key {
            return Err(Refusal::new(
                RefusalReason::WrongSpecialist,
                format!("this specialist is {}", self.key),
            )
            .into());
        }
        let refused = |reason, detail: &str| Err(Refusal::new(reason, detail).into());
        if r.issued > now + CLOCK_SKEW {
            return refused(
                RefusalReason::NotYetValid,
                "issued ahead of this host's clock",
            );
        }
        if r.issued < now - CLOCK_SKEW {
            return refused(RefusalReason::Expired, "issued more than 5 minutes ago");
        }
        if !well_formed_nonce(&r.nonce) {
            return refused(
                RefusalReason::InvalidEnvelope,
                "nonce is not 128 bits of base64url",
            );
        }
        if self.revocation_nonces.seen(&r.orchestrator, &r.nonce, now) {
            return refused(RefusalReason::Replayed, "nonce already received");
        }
        if self
            .binding
            .as_ref()
            .is_none_or(|b| b.orchestrator != r.orchestrator)
        {
            return refused(
                RefusalReason::NotBoundOrchestrator,
                "only the bound orchestrator revokes",
            );
        }
        self.revocation_nonces
            .record(&r.orchestrator, &r.nonce, now + REVOCATION_NONCE_TTL, now);
        let dropped = self
            .active()
            .is_some_and(|d| r.flight_id.is_none_or(|flight| flight == d.flight_id));
        if dropped {
            self.drop_active()?;
        }
        if r.release_binding {
            self.release()?;
        }
        Ok((
            Revoked {
                status: RevokedStatus::Revoked,
                specialist: self.key.clone(),
            },
            dropped || r.release_binding,
        ))
    }

    /// Drop the active delegation because this host can no longer take
    /// it, such as after a restart on a host that lost a capability.
    pub fn abandon(&mut self) -> Result<(), StoreError> {
        self.drop_active()
    }

    /// Drop the active delegation if it has expired; whether it did.
    pub fn expire(&mut self, now: DateTime<Utc>) -> Result<bool, StoreError> {
        if self.active().is_some_and(|d| d.expires <= now) {
            self.drop_active()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn drop_active(&mut self) -> Result<(), StoreError> {
        if self.active.take().is_some() {
            self.store.save_delegation(None)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
