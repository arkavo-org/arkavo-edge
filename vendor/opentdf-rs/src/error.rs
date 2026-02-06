//! Unified error type for OpenTDF public API
//!
//! Internal modules maintain their domain-specific errors for precise handling.
//! This unified type provides a clean public API for SDK consumers.
//!
//! # Example
//!
//! ```no_run
//! use opentdf::OpenTdfError;
//!
//! fn process_tdf() -> Result<(), OpenTdfError> {
//!     // All TDF operations return OpenTdfError
//!     // Internal error types are automatically converted
//!     Ok(())
//! }
//! ```

use thiserror::Error;

/// Unified error type for all OpenTDF operations
///
/// This error type consolidates all domain-specific errors into a single type
/// for convenient error handling by SDK consumers. Advanced users who need
/// more granular error handling can still access the underlying error types
/// through the enum variants.
///
/// # Error Categories
///
/// - **Policy**: Policy validation, parsing, or evaluation errors
/// - **Archive**: TDF archive creation, reading, or format errors
/// - **Kas**: Key Access Service communication or protocol errors
/// - **Crypto**: Encryption, decryption, or key generation errors
/// - **Hmac**: HMAC calculation or verification errors
/// - **Key**: Key format, parsing, or validation errors
#[derive(Debug, Error)]
pub enum OpenTdfError {
    /// Policy validation or evaluation error
    #[error("Policy error: {0}")]
    Policy(#[from] crate::policy::PolicyError),

    /// TDF archive operations error
    #[error("Archive error: {0}")]
    Archive(#[from] crate::archive::TdfError),

    /// Key Access Service error
    #[error("KAS error: {0}")]
    Kas(#[from] opentdf_protocol::KasError),

    /// Cryptographic operation error
    #[error("Crypto error: {0}")]
    Crypto(#[from] opentdf_crypto::EncryptionError),

    /// HMAC/signature error
    #[error("HMAC error: {0}")]
    Hmac(#[from] opentdf_crypto::hmac::HmacError),

    /// Key format or validation error
    #[error("Key error: {0}")]
    Key(#[from] opentdf_crypto::KeyError),
}

impl OpenTdfError {
    /// Returns true if the error is potentially retryable
    ///
    /// Network-related errors (KAS communication) and temporary failures
    /// may be retryable after a delay.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Kas(e) => e.is_retryable(),
            Self::Archive(e) => e.is_retryable(),
            _ => false,
        }
    }

    /// Returns a suggestion for resolving this error
    ///
    /// Provides user-friendly guidance when available.
    pub fn suggestion(&self) -> Option<&str> {
        match self {
            Self::Kas(e) => e.suggestion(),
            Self::Archive(e) => e.suggestion(),
            Self::Policy(e) => e.suggestion(),
            _ => None,
        }
    }

    /// Returns true if this is a policy-related error
    pub fn is_policy_error(&self) -> bool {
        matches!(self, Self::Policy(_))
    }

    /// Returns true if this is a cryptographic error
    pub fn is_crypto_error(&self) -> bool {
        matches!(self, Self::Crypto(_) | Self::Hmac(_) | Self::Key(_))
    }

    /// Returns true if this is a KAS communication error
    pub fn is_kas_error(&self) -> bool {
        matches!(self, Self::Kas(_))
    }

    /// Returns true if this is an archive format error
    pub fn is_archive_error(&self) -> bool {
        matches!(self, Self::Archive(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_categories() {
        // Test policy error categorization
        let policy_err = OpenTdfError::Policy(crate::policy::PolicyError::InvalidAttribute {
            fqn: "test".to_string(),
            reason: "test reason".to_string(),
        });
        assert!(policy_err.is_policy_error());
        assert!(!policy_err.is_crypto_error());
        assert!(!policy_err.is_kas_error());
        assert!(!policy_err.is_archive_error());

        // Test archive error categorization
        let archive_err =
            OpenTdfError::Archive(crate::archive::TdfError::MissingRequiredField { field: "test" });
        assert!(archive_err.is_archive_error());
        assert!(!archive_err.is_policy_error());
    }

    #[test]
    fn test_error_display() {
        let err = OpenTdfError::Policy(crate::policy::PolicyError::InvalidAttribute {
            fqn: "test".to_string(),
            reason: "test reason".to_string(),
        });
        let msg = err.to_string();
        assert!(msg.contains("Policy error"));
    }
}
