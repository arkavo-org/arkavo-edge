//! SwarmKit delegation (spec draft-02 §7): an orchestrator hands a role to a
//! specialist in a signed envelope, and the specialist accepts it only from
//! the orchestrator it is bound to.
//!
//! The orchestrator side builds and signs envelopes and revocations
//! ([`Delegation::for_role`], [`Delegation::sign`], [`Revocation::sign`]).
//! The specialist side ([`Specialist`]) checks them in the order §7.3 sets,
//! keeps its binding and active delegation on disk so a restart keeps its
//! role, and reports its status. Applying a role to a running agent is the
//! caller's job; this crate decides only whether to accept it.
//!
//! Signatures cover the JSON value as sent, not a re-serialization of the
//! parsed structs, so fields the two sides model differently can never make
//! a valid envelope fail or an altered one pass.

mod acceptance;
mod envelope;
#[cfg(test)]
mod fixture;
mod protocol;
mod revocation;
mod signing;
mod specialist;
mod store;

pub use acceptance::NonceCache;
pub use envelope::{
    BuildError, Delegation, DelegationContext, MAX_LIFETIME, McpGrant, SignedDelegation,
};
pub use protocol::{
    Binding, BindingMode, CapabilityReport, DelegationSummary, REFUSAL_CODE, Ready, Refusal,
    RefusalReason, Revoked, SpecialistStatus, methods,
};
pub use revocation::{Revocation, SignedRevocation};
pub use signing::{ALGORITHM, digest_b64};
pub use specialist::{Admitted, Policy, RevokeError, Specialist};
pub use store::{StateStore, StoreError};
