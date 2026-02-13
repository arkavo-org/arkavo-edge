//! Error Sanitization
//!
//! Separates internal error details from external-facing messages.
//!
//! ## Spec Coverage
//! - [specs/arkavo-edge/session-security.spec.yaml](SESS-013): Internal errors not exposed externally
//! - [specs/arkavo-edge/session-security.spec.yaml](SESS-014): Security errors with appropriate detail levels
//! - [specs/arkavo-edge/session-security.spec.yaml](SESS-015): Error chain preserves security context

use std::collections::HashMap;

/// Error classification for determining external exposure
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ErrorClassification {
    /// Safe to expose externally (e.g., "Invalid input format")
    SafeExternal,
    /// Internal only - contains sensitive details
    InternalOnly,
    /// Security-related - generic external message
    SecuritySensitive,
}

/// Sanitized error for external consumption
#[derive(Debug, Clone)]
pub struct ExternalError {
    /// Generic error message safe for clients
    pub message: String,
    /// Error code for programmatic handling
    pub code: String,
    /// Correlation ID for log tracing
    pub correlation_id: String,
}

impl ExternalError {
    /// Creates a generic authentication error
    pub fn authentication_failed(correlation_id: impl Into<String>) -> Self {
        Self {
            message: "Authentication failed".to_string(),
            code: "AUTH_FAILED".to_string(),
            correlation_id: correlation_id.into(),
        }
    }

    /// Creates a generic authorization error
    pub fn unauthorized(correlation_id: impl Into<String>) -> Self {
        Self {
            message: "Unauthorized".to_string(),
            code: "UNAUTHORIZED".to_string(),
            correlation_id: correlation_id.into(),
        }
    }

    /// Creates a generic internal error
    pub fn internal_error(correlation_id: impl Into<String>) -> Self {
        Self {
            message: "Internal error".to_string(),
            code: "INTERNAL_ERROR".to_string(),
            correlation_id: correlation_id.into(),
        }
    }
}

/// Internal error with full details
#[derive(Debug, Clone)]
pub struct InternalError {
    /// Full technical details
    pub details: String,
    /// Source/error chain
    pub source: Option<Box<InternalError>>,
    /// Classification
    pub classification: ErrorClassification,
    /// Correlation ID
    pub correlation_id: String,
}

/// Error sanitizer that manages internal vs external error exposure
#[derive(Debug)]
pub struct ErrorSanitizer {
    correlation_prefix: String,
    counter: std::sync::atomic::AtomicU64,
}

impl ErrorSanitizer {
    /// Creates a new error sanitizer
    pub fn new() -> Self {
        Self {
            correlation_prefix: "err".to_string(),
            counter: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// Generates a new correlation ID
    pub fn generate_correlation_id(&self) -> String {
        let count = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        format!("{}-{:016x}", self.correlation_prefix, count)
    }

    /// Sanitizes an internal error for external exposure
    ///
    /// ## Spec
    /// SESS-013: Internal errors not exposed externally
    /// SESS-014: Security errors have appropriate detail levels
    /// SESS-015: Error chain preserves security context
    pub fn sanitize(&self, internal: &InternalError) -> ExternalError {
        // Determine classification - use most specific from chain (SESS-015)
        let classification = self.determine_classification(internal);

        match classification {
            ErrorClassification::SafeExternal => {
                // SESS-014: Safe errors can include helpful details
                ExternalError {
                    message: internal.details.clone(),
                    code: "VALIDATION_ERROR".to_string(),
                    correlation_id: internal.correlation_id.clone(),
                }
            }
            ErrorClassification::SecuritySensitive => {
                // SESS-014: Security errors show generic external message
                ExternalError::authentication_failed(&internal.correlation_id)
            }
            ErrorClassification::InternalOnly => {
                // SESS-013: Internal errors not exposed externally
                ExternalError::internal_error(&internal.correlation_id)
            }
        }
    }

    /// Determines the most specific error classification from the chain
    fn determine_classification(&self, internal: &InternalError) -> ErrorClassification {
        // Start with current error's classification
        let mut classification = internal.classification;

        // Walk the chain to find most specific (SESS-015)
        let mut current = internal.source.as_ref();
        while let Some(source) = current {
            // Security-sensitive errors take precedence
            if source.classification == ErrorClassification::SecuritySensitive {
                classification = ErrorClassification::SecuritySensitive;
            }
            current = source.source.as_ref();
        }

        classification
    }

    /// Classifies an error type
    pub fn classify_error_type(error_type: &str) -> ErrorClassification {
        match error_type {
            "ValidationError" | "ParseError" | "InvalidFormat" => ErrorClassification::SafeExternal,
            "AuthError" | "PermissionDenied" | "RevokedError" => {
                ErrorClassification::SecuritySensitive
            }
            _ => ErrorClassification::InternalOnly,
        }
    }
}

impl Default for ErrorSanitizer {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for errors that can be sanitized
pub trait SanitizableError {
    /// Returns the error classification
    fn classification(&self) -> ErrorClassification;

    /// Converts to external error representation
    fn to_external(&self, correlation_id: &str) -> ExternalError;

    /// Returns safe external message
    fn external_message(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    //! TDD Tests for Error Sanitization

    use super::*;

    // ============================================================================
    // SESS-013: Internal errors not exposed externally
    // ============================================================================

    /// Test: Database error details not exposed externally
    /// Spec: SESS-013 - Internal errors not exposed externally
    #[test]
    fn test_database_error_not_exposed() {
        // Arrange
        let sanitizer = ErrorSanitizer::new();
        let internal = InternalError {
            details:
                "PostgreSQL connection failed: password authentication failed for user 'admin'"
                    .to_string(),
            source: None,
            classification: ErrorClassification::InternalOnly,
            correlation_id: sanitizer.generate_correlation_id(),
        };

        // Act
        let external = sanitizer.sanitize(&internal);

        // Assert
        assert!(!external.message.contains("PostgreSQL"));
        assert!(!external.message.contains("password"));
        assert!(!external.message.contains("admin"));
        assert_eq!(external.code, "INTERNAL_ERROR");
        assert!(!external.correlation_id.is_empty());
    }

    /// Test: Stack traces not exposed externally
    /// Spec: SESS-013 - Stack traces are internal only
    #[test]
    fn test_stack_trace_not_exposed() {
        // Arrange
        let sanitizer = ErrorSanitizer::new();
        let internal = InternalError {
            details: "panicked at 'index out of bounds', src/lib.rs:42:10".to_string(),
            source: None,
            classification: ErrorClassification::InternalOnly,
            correlation_id: "test-123".to_string(),
        };

        // Act
        let external = sanitizer.sanitize(&internal);

        // Assert
        assert!(!external.message.contains("panicked"));
        assert!(!external.message.contains("src/lib.rs"));
        assert!(!external.message.contains("index out of bounds"));
    }

    /// Test: Correlation ID links internal and external errors
    /// Spec: SESS-013 - Error correlation ID provided
    #[test]
    fn test_correlation_id_links_errors() {
        // Arrange
        let sanitizer = ErrorSanitizer::new();
        let correlation_id = sanitizer.generate_correlation_id();
        let internal = InternalError {
            details: "Something went wrong".to_string(),
            source: None,
            classification: ErrorClassification::InternalOnly,
            correlation_id: correlation_id.clone(),
        };

        // Act
        let external = sanitizer.sanitize(&internal);

        // Assert
        assert_eq!(external.correlation_id, correlation_id);
    }

    // ============================================================================
    // SESS-014: Security errors have appropriate detail levels
    // ============================================================================

    /// Test: Auth errors show generic external message
    /// Spec: SESS-014 - External: Generic "authentication failed"
    #[test]
    fn test_auth_error_generic_external() {
        // Arrange
        let sanitizer = ErrorSanitizer::new();
        let internal = InternalError {
            details: "Password mismatch for user 'john' - hash comparison failed".to_string(),
            source: None,
            classification: ErrorClassification::SecuritySensitive,
            correlation_id: "corr-123".to_string(),
        };

        // Act
        let external = sanitizer.sanitize(&internal);

        // Assert
        assert_eq!(external.message, "Authentication failed");
        assert_eq!(external.code, "AUTH_FAILED");
    }

    /// Test: Token expired shows generic message
    /// Spec: SESS-014 - Different auth failures show same external message
    #[test]
    fn test_token_expired_generic_external() {
        // Arrange
        let sanitizer = ErrorSanitizer::new();
        let internal = InternalError {
            details: "Token expired at 2024-01-01T00:00:00Z".to_string(),
            source: None,
            classification: ErrorClassification::SecuritySensitive,
            correlation_id: "corr-456".to_string(),
        };

        // Act
        let external = sanitizer.sanitize(&internal);

        // Assert
        assert_eq!(external.message, "Authentication failed");
        assert!(!external.message.contains("expired")); // Don't reveal reason
    }

    /// Test: Safe errors can have detailed external messages
    /// Spec: SESS-014 - Safe errors preserve helpful details
    #[test]
    fn test_safe_error_preserves_details() {
        // Arrange
        let sanitizer = ErrorSanitizer::new();
        let internal = InternalError {
            details: "Invalid JSON: unexpected character at line 5, column 12".to_string(),
            source: None,
            classification: ErrorClassification::SafeExternal,
            correlation_id: "corr-789".to_string(),
        };

        // Act
        let external = sanitizer.sanitize(&internal);

        // Assert
        // For safe errors, we can include helpful details
        assert!(external.message.contains("Invalid") || external.code.contains("VALIDATION"));
    }

    // ============================================================================
    // SESS-015: Error chain preserves security context
    // ============================================================================

    /// Test: Error chain uses most specific security error
    /// Spec: SESS-015 - Most specific security error determines external message
    #[test]
    fn test_error_chain_uses_most_specific() {
        // Arrange
        let sanitizer = ErrorSanitizer::new();
        let root = InternalError {
            details: "Database connection pool exhausted".to_string(),
            source: None,
            classification: ErrorClassification::InternalOnly,
            correlation_id: "root-1".to_string(),
        };
        let auth_error = InternalError {
            details: "Failed to validate credentials".to_string(),
            source: Some(Box::new(root)),
            classification: ErrorClassification::SecuritySensitive,
            correlation_id: "auth-1".to_string(),
        };

        // Act
        let external = sanitizer.sanitize(&auth_error);

        // Assert
        // Should use auth error classification, not root
        assert_eq!(external.message, "Authentication failed");
    }

    /// Test: Validation errors preserve field info
    /// Spec: SESS-015 - Validation context preserved for safe errors
    #[test]
    fn test_validation_error_preserves_context() {
        // Arrange
        let sanitizer = ErrorSanitizer::new();
        let internal = InternalError {
            details: "Field 'email' is required".to_string(),
            source: None,
            classification: ErrorClassification::SafeExternal,
            correlation_id: "val-1".to_string(),
        };

        // Act
        let external = sanitizer.sanitize(&internal);

        // Assert
        // Safe errors can include field information
        assert!(
            external.message.to_lowercase().contains("field")
                || external.code.contains("VALIDATION")
        );
    }

    // ============================================================================
    // Classification tests
    // ============================================================================

    #[test]
    fn test_error_classification_validation() {
        assert_eq!(
            ErrorSanitizer::classify_error_type("ValidationError"),
            ErrorClassification::SafeExternal
        );
    }

    #[test]
    fn test_error_classification_auth() {
        assert_eq!(
            ErrorSanitizer::classify_error_type("AuthError"),
            ErrorClassification::SecuritySensitive
        );
    }

    #[test]
    fn test_error_classification_unknown() {
        assert_eq!(
            ErrorSanitizer::classify_error_type("UnknownError"),
            ErrorClassification::InternalOnly
        );
    }

    #[test]
    fn test_external_error_factory_methods() {
        let auth = ExternalError::authentication_failed("corr-1");
        assert_eq!(auth.message, "Authentication failed");
        assert_eq!(auth.code, "AUTH_FAILED");

        let unauth = ExternalError::unauthorized("corr-2");
        assert_eq!(unauth.message, "Unauthorized");
        assert_eq!(unauth.code, "UNAUTHORIZED");

        let internal = ExternalError::internal_error("corr-3");
        assert_eq!(internal.message, "Internal error");
        assert_eq!(internal.code, "INTERNAL_ERROR");
    }

    #[test]
    fn test_correlation_id_generation() {
        let sanitizer = ErrorSanitizer::new();
        let id1 = sanitizer.generate_correlation_id();
        let id2 = sanitizer.generate_correlation_id();

        assert_ne!(id1, id2);
        assert!(id1.starts_with("err-"));
        assert!(id2.starts_with("err-"));
    }
}
