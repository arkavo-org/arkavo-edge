#[derive(Debug, Clone)]
pub enum SizeValidationError {
    TooLarge {
        label: String,
        actual: usize,
        max: usize,
    },
}

impl std::fmt::Display for SizeValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SizeValidationError::TooLarge { label, actual, max } => {
                write!(
                    f,
                    "{label} exceeds maximum size: {actual} bytes > {max} bytes"
                )
            }
        }
    }
}

impl std::error::Error for SizeValidationError {}

/// Validate that a string does not exceed a maximum byte size.
pub fn validate_str_size(
    value: &str,
    max_bytes: usize,
    label: &str,
) -> Result<(), SizeValidationError> {
    let actual = value.len();
    if actual > max_bytes {
        return Err(SizeValidationError::TooLarge {
            label: label.to_string(),
            actual,
            max: max_bytes,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Unit tests for size validation to prevent DoS via resource exhaustion.
    //!
    //! ## Spec Coverage
    //! - [specs/arkavo-edge/network-security.spec.yaml](NET-010): Rate limiting per IP (related resource control)
    //! - [specs/arkavo-edge/protocol.spec.yaml](PROTO-007): Rate limiting enforcement

    use super::*;
    use arkavo_test_macros::spec;

    #[spec("VAL-004")]
    #[test]
    fn test_within_limit() {
        assert!(validate_str_size("hello", 10, "test").is_ok());
    }

    #[spec("VAL-004")]
    #[test]
    fn test_at_limit() {
        assert!(validate_str_size("hello", 5, "test").is_ok());
    }

    #[spec("VAL-004")]
    #[test]
    fn test_exceeds_limit() {
        let result = validate_str_size("hello world", 5, "payload");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("payload"));
        assert!(err.to_string().contains("11 bytes"));
    }
}
