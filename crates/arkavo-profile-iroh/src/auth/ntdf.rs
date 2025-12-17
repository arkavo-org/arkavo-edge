//! NTDF token validation.
//!
//! This module validates NTDF authentication tokens.
//! In production, this would decrypt and verify the NanoTDF token
//! using the KAS private key. For now, we implement a simplified
//! validation that checks token format.

use thiserror::Error;
use tracing::debug;

/// NTDF authentication errors.
#[derive(Debug, Error)]
pub enum NtdfAuthError {
    #[error("Invalid token format")]
    InvalidFormat,
    #[error("Token expired")]
    TokenExpired,
    #[error("Invalid audience: expected {expected}, got {actual}")]
    InvalidAudience { expected: String, actual: String },
    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),
}

/// Validate an NTDF token.
///
/// # Arguments
/// * `token` - Z85-encoded NTDF token
/// * `expected_audience` - Expected audience claim
///
/// # Returns
/// * `Ok(())` if token is valid
/// * `Err(NtdfAuthError)` if validation fails
pub fn validate_ntdf_token(token: &str, expected_audience: &str) -> Result<(), NtdfAuthError> {
    debug!(
        "Validating NTDF token (length: {})",
        token.len()
    );

    // Basic format validation
    if token.is_empty() {
        return Err(NtdfAuthError::InvalidFormat);
    }

    // In a full implementation, this would:
    // 1. Decode Z85 token
    // 2. Parse NanoTDF header
    // 3. Decrypt using KAS private key
    // 4. Validate claims (exp, aud, etc.)

    // For now, we accept any non-empty token in development
    // TODO: Implement full NTDF token validation when KAS integration is complete

    debug!("NTDF token validated for audience: {}", expected_audience);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_token() {
        let result = validate_ntdf_token("", "https://iroh.arkavo.net");
        assert!(matches!(result, Err(NtdfAuthError::InvalidFormat)));
    }

    #[test]
    fn test_valid_token() {
        // Any non-empty token is valid in development mode
        let result = validate_ntdf_token("test-token", "https://iroh.arkavo.net");
        assert!(result.is_ok());
    }
}
