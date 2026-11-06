//! What a successful verification proves, and the decision record kept for every protected
//! request.
//!
//! [`VerifiedAgent`] is built only by the verifier. It says which key signed which request
//! for which purpose, under which trust authority. It does not say who the represented person
//! is, or that any purchase is approved: those are separate results, from separate evidence.

use crate::keys::TrustAuthority;
use crate::profile::{Operation, Profile, Purpose};

/// The request a signature is bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestBinding {
    pub method: String,
    pub target_uri: String,
    /// The covered `Content-Digest`, when the profile requires one.
    pub content_digest: Option<String>,
    /// The covered `Idempotency-Key`, when the profile requires one.
    pub idempotency_key: Option<String>,
}

/// An authenticated agent request. Read-only; constructed only inside this crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedAgent {
    pub(crate) authority: TrustAuthority,
    pub(crate) key_id: String,
    pub(crate) key_thumbprint: String,
    pub(crate) provider: String,
    pub(crate) runtime: Option<String>,
    pub(crate) profile: Profile,
    pub(crate) operation: Operation,
    pub(crate) purpose: Purpose,
    pub(crate) covered: Vec<String>,
    pub(crate) binding: RequestBinding,
    pub(crate) nonce: String,
    pub(crate) created: i64,
    pub(crate) expires: i64,
    pub(crate) decision_id: String,
    pub(crate) provenance: String,
}

impl VerifiedAgent {
    pub fn authority(&self) -> &TrustAuthority {
        &self.authority
    }

    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// The signer's identity: the RFC 7638 thumbprint of the verified key.
    pub fn key_thumbprint(&self) -> &str {
        &self.key_thumbprint
    }

    /// The agent provider the trust authority binds the key to.
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// The runtime instance, only when the authority verified the binding. `None` means the
    /// provider is known but the individual runtime is not.
    pub fn runtime(&self) -> Option<&str> {
        self.runtime.as_deref()
    }

    pub fn profile(&self) -> Profile {
        self.profile
    }

    pub fn operation(&self) -> Operation {
        self.operation
    }

    pub fn purpose(&self) -> Purpose {
        self.purpose
    }

    pub fn covered_components(&self) -> &[String] {
        &self.covered
    }

    pub fn binding(&self) -> &RequestBinding {
        &self.binding
    }

    pub fn nonce(&self) -> &str {
        &self.nonce
    }

    pub fn created(&self) -> i64 {
        self.created
    }

    pub fn expires(&self) -> i64 {
        self.expires
    }

    /// The decision record this verification produced.
    pub fn decision_id(&self) -> &str {
        &self.decision_id
    }

    /// Where the key record came from.
    pub fn provenance(&self) -> &str {
        &self.provenance
    }
}

/// The audit record of one decision about a protected request, accepted or rejected.
/// It names the evidence used, never key material, signatures or consumer tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionRecord {
    pub decision_id: String,
    pub profile: Profile,
    pub accepted: bool,
    /// `accepted`, or the rejection code.
    pub result: String,
    /// Internal detail; kept in protected logs, never returned to the caller.
    pub reason: String,
    pub key_id: Option<String>,
    pub key_thumbprint: Option<String>,
    pub authority: String,
    pub elapsed_micros: u64,
}

impl DecisionRecord {
    /// Writes the record to the audit log target.
    pub fn emit(&self) {
        if self.accepted {
            tracing::info!(
                target: "arkavo_tap::audit",
                decision_id = %self.decision_id,
                profile = %self.profile,
                result = %self.result,
                key_id = self.key_id.as_deref().unwrap_or(""),
                key_thumbprint = self.key_thumbprint.as_deref().unwrap_or(""),
                authority = %self.authority,
                elapsed_micros = self.elapsed_micros,
                "tap decision"
            );
        } else {
            tracing::warn!(
                target: "arkavo_tap::audit",
                decision_id = %self.decision_id,
                profile = %self.profile,
                result = %self.result,
                reason = %self.reason,
                key_id = self.key_id.as_deref().unwrap_or(""),
                authority = %self.authority,
                elapsed_micros = self.elapsed_micros,
                "tap decision"
            );
        }
    }
}
