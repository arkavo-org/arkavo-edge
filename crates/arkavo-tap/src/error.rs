//! Why a protected request was refused, and the HTTP status each reason maps to.
//!
//! The mapping is Arkavo's API contract, not a TAP error registry. Callers get the kind and a
//! correlation ID; the detail stays in protected logs, because key-resolution and policy
//! diagnostics tell an attacker which check to aim at next.

use std::fmt;
use thiserror::Error;

/// The category of a refusal. Each maps to one HTTP status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RejectKind {
    /// A signature field that does not parse, is not in canonical form, or is ambiguous.
    Malformed,
    /// A protected request without the proof its route requires.
    MissingProof,
    /// A signature that does not verify, or a digest that does not match the content.
    InvalidSignature,
    /// A signature outside its time bounds, or one whose lifetime is longer than allowed.
    Expired,
    /// A key the configured trust authority does not vouch for, or a revoked or wrong-type key.
    UntrustedActor,
    /// A declared purpose (tag) that the route does not accept.
    PurposeDenied,
    /// No purchase authority, or authority that does not cover this exact operation.
    AuthorityDenied,
    /// A nonce already used within its scope.
    Replay,
    /// An idempotency key reused for a different request.
    IdempotencyConflict,
    /// Content larger than the route allows.
    ContentTooLarge,
    /// Header fields larger than the verifier allows.
    HeadersTooLarge,
    /// Too many verifications or key lookups.
    RateLimited,
    /// A trust, replay or authorization dependency that cannot answer: fail closed.
    Unavailable,
}

impl RejectKind {
    /// The HTTP status for this kind.
    pub fn status(self) -> u16 {
        match self {
            RejectKind::Malformed => 400,
            RejectKind::MissingProof | RejectKind::InvalidSignature | RejectKind::Expired => 401,
            RejectKind::UntrustedActor
            | RejectKind::PurposeDenied
            | RejectKind::AuthorityDenied => 403,
            RejectKind::Replay | RejectKind::IdempotencyConflict => 409,
            RejectKind::ContentTooLarge => 413,
            RejectKind::HeadersTooLarge => 431,
            RejectKind::RateLimited => 429,
            RejectKind::Unavailable => 503,
        }
    }

    /// A stable machine-readable code for responses and audit records.
    pub fn code(self) -> &'static str {
        match self {
            RejectKind::Malformed => "malformed",
            RejectKind::MissingProof => "missing_proof",
            RejectKind::InvalidSignature => "invalid_signature",
            RejectKind::Expired => "expired",
            RejectKind::UntrustedActor => "untrusted_actor",
            RejectKind::PurposeDenied => "purpose_denied",
            RejectKind::AuthorityDenied => "authority_denied",
            RejectKind::Replay => "replay",
            RejectKind::IdempotencyConflict => "idempotency_conflict",
            RejectKind::ContentTooLarge => "content_too_large",
            RejectKind::HeadersTooLarge => "headers_too_large",
            RejectKind::RateLimited => "rate_limited",
            RejectKind::Unavailable => "unavailable",
        }
    }
}

impl fmt::Display for RejectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// A refusal: its kind, for the caller, and a detail for protected logs only.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{kind}: {detail}")]
pub struct Rejection {
    pub kind: RejectKind,
    pub detail: String,
}

impl Rejection {
    pub fn new(kind: RejectKind, detail: impl Into<String>) -> Self {
        Rejection {
            kind,
            detail: detail.into(),
        }
    }

    pub fn malformed(detail: impl Into<String>) -> Self {
        Self::new(RejectKind::Malformed, detail)
    }
}

/// Errors from building or signing an outbound request.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SignError {
    /// The signing policy does not allow this destination or operation.
    #[error("signing refused: {0}")]
    Refused(String),
    /// The request cannot be represented as the profile requires.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_the_contract_status() {
        let cases = [
            (RejectKind::Malformed, 400),
            (RejectKind::MissingProof, 401),
            (RejectKind::InvalidSignature, 401),
            (RejectKind::Expired, 401),
            (RejectKind::UntrustedActor, 403),
            (RejectKind::PurposeDenied, 403),
            (RejectKind::AuthorityDenied, 403),
            (RejectKind::Replay, 409),
            (RejectKind::IdempotencyConflict, 409),
            (RejectKind::ContentTooLarge, 413),
            (RejectKind::HeadersTooLarge, 431),
            (RejectKind::RateLimited, 429),
            (RejectKind::Unavailable, 503),
        ];
        for (kind, status) in cases {
            assert_eq!(kind.status(), status, "{kind}");
            assert_ne!(kind.code(), "");
        }
    }

    #[test]
    fn display_shows_kind_and_detail() {
        let r = Rejection::malformed("two labels");
        assert_eq!(r.to_string(), "malformed: two labels");
    }
}
