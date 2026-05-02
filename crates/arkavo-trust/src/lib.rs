pub mod events;
pub mod jcs;
pub mod mapper;
pub mod service;
pub mod signing;
pub mod store;
pub mod types;

pub use mapper::{AgentTrustInput, compute_trust_score};
pub use service::{SharedTrustService, TrustService};
pub use signing::{
    SigningError, sign_trust_event, sign_trust_score, verify_trust_event, verify_trust_score,
};
pub use types::*;

/// Errors returned by MCP-T operations.
#[derive(Debug, thiserror::Error)]
pub enum TrustError {
    #[error("Subject not found: {0}")]
    SubjectNotFound(String),
    #[error("Invalid signature: {0}")]
    InvalidSignature(String),
    #[error("Duplicate event: {0}")]
    DuplicateEvent(String),
    #[error("Domain not supported: {0}")]
    DomainNotSupported(String),
    #[error("Stale score for subject: {0}")]
    StaleScore(String),
}

impl TrustError {
    /// Map to MCP-T JSON-RPC error code.
    pub fn error_code(&self) -> i32 {
        match self {
            Self::SubjectNotFound(_) => error_codes::SUBJECT_NOT_FOUND,
            Self::InvalidSignature(_) => error_codes::INVALID_SIGNATURE,
            Self::DuplicateEvent(_) => error_codes::DUPLICATE_EVENT,
            Self::DomainNotSupported(_) => error_codes::DOMAIN_NOT_SUPPORTED,
            Self::StaleScore(_) => error_codes::STALE_SCORE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_match_spec() {
        assert_eq!(TrustError::SubjectNotFound("x".into()).error_code(), -32001);
        assert_eq!(
            TrustError::InvalidSignature("x".into()).error_code(),
            -32010
        );
        assert_eq!(TrustError::DuplicateEvent("x".into()).error_code(), -32012);
        assert_eq!(
            TrustError::DomainNotSupported("x".into()).error_code(),
            -32003
        );
        assert_eq!(TrustError::StaleScore("x".into()).error_code(), -32004);
    }
}
