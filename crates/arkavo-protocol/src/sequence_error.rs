use std::fmt;

/// SEQ-017: Error types for sequence integrity system
#[derive(Debug)]
pub enum SequenceIntegrityError {
    StorageFailure { source: String },
    TrackingTimeout { action: String },
    TaintBridgeUnavailable,
    LedgerCorrupted { detail: String },
}

impl fmt::Display for SequenceIntegrityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sequence integrity error")
    }
}

impl std::error::Error for SequenceIntegrityError {}

/// Policy for how errors affect agent operation
#[derive(Debug, PartialEq)]
pub enum ErrorPolicy {
    /// Continue operation, log the error
    ContinueWithLogging,
    /// Block high-consequence actions only
    BlockHighConsequence,
    /// Block all actions until recovery
    BlockAll,
}

impl SequenceIntegrityError {
    /// Determine error policy based on error type
    pub fn policy(&self) -> ErrorPolicy {
        ErrorPolicy::ContinueWithLogging
    }

    /// Attempt recovery from the error
    pub fn can_recover(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // SEQ-017: Handle sequence tracking errors gracefully
    // =========================================================================

    #[test]
    fn storage_failure_allows_reads_but_blocks_writes() {
        let err = SequenceIntegrityError::StorageFailure {
            source: "sqlite".into(),
        };
        assert_eq!(err.policy(), ErrorPolicy::BlockHighConsequence);
    }

    #[test]
    fn tracking_timeout_continues_with_logging() {
        let err = SequenceIntegrityError::TrackingTimeout {
            action: "read_file".into(),
        };
        assert_eq!(err.policy(), ErrorPolicy::ContinueWithLogging);
    }

    #[test]
    fn taint_bridge_unavailable_blocks_high_consequence() {
        let err = SequenceIntegrityError::TaintBridgeUnavailable;
        assert_eq!(err.policy(), ErrorPolicy::BlockHighConsequence);
    }

    #[test]
    fn ledger_corruption_blocks_all() {
        let err = SequenceIntegrityError::LedgerCorrupted {
            detail: "checksum mismatch".into(),
        };
        assert_eq!(err.policy(), ErrorPolicy::BlockAll);
    }

    #[test]
    fn storage_failure_can_recover() {
        let err = SequenceIntegrityError::StorageFailure {
            source: "sqlite".into(),
        };
        assert!(err.can_recover());
    }

    #[test]
    fn ledger_corruption_cannot_recover() {
        let err = SequenceIntegrityError::LedgerCorrupted {
            detail: "data loss".into(),
        };
        assert!(!err.can_recover());
    }

    #[test]
    fn error_displays_meaningful_message() {
        let err = SequenceIntegrityError::StorageFailure {
            source: "sqlite".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("sqlite"));
    }
}
