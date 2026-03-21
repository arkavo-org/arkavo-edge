use std::fmt;

#[derive(Debug)]
pub enum SequenceIntegrityError {
    StorageFailure { source: String },
    TrackingTimeout { action: String },
    TaintBridgeUnavailable,
    LedgerCorrupted { detail: String },
}

impl fmt::Display for SequenceIntegrityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StorageFailure { source } => {
                write!(f, "storage failure: {source}")
            }
            Self::TrackingTimeout { action } => {
                write!(f, "tracking timeout on action: {action}")
            }
            Self::TaintBridgeUnavailable => {
                write!(f, "taint bridge unavailable")
            }
            Self::LedgerCorrupted { detail } => {
                write!(f, "ledger corrupted: {detail}")
            }
        }
    }
}

impl std::error::Error for SequenceIntegrityError {}

#[derive(Debug, PartialEq)]
pub enum ErrorPolicy {
    ContinueWithLogging,
    BlockHighConsequence,
    BlockAll,
}

impl SequenceIntegrityError {
    pub fn policy(&self) -> ErrorPolicy {
        match self {
            Self::TrackingTimeout { .. } => ErrorPolicy::ContinueWithLogging,
            Self::StorageFailure { .. } | Self::TaintBridgeUnavailable => {
                ErrorPolicy::BlockHighConsequence
            }
            Self::LedgerCorrupted { .. } => ErrorPolicy::BlockAll,
        }
    }

    pub fn can_recover(&self) -> bool {
        !matches!(self, Self::LedgerCorrupted { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("SEQ-017")]
    #[test]
    fn storage_failure_blocks_high_consequence_actions() {
        let err = SequenceIntegrityError::StorageFailure {
            source: "sqlite".into(),
        };
        assert_eq!(err.policy(), ErrorPolicy::BlockHighConsequence);
    }

    #[spec("SEQ-017")]
    #[test]
    fn tracking_timeout_continues_with_logging() {
        let err = SequenceIntegrityError::TrackingTimeout {
            action: "read_file".into(),
        };
        assert_eq!(err.policy(), ErrorPolicy::ContinueWithLogging);
    }

    #[spec("SEQ-017")]
    #[test]
    fn taint_bridge_unavailable_blocks_high_consequence() {
        let err = SequenceIntegrityError::TaintBridgeUnavailable;
        assert_eq!(err.policy(), ErrorPolicy::BlockHighConsequence);
    }

    #[spec("SEQ-017")]
    #[test]
    fn ledger_corruption_blocks_all_actions() {
        let err = SequenceIntegrityError::LedgerCorrupted {
            detail: "checksum mismatch".into(),
        };
        assert_eq!(err.policy(), ErrorPolicy::BlockAll);
    }

    #[spec("SEQ-017")]
    #[test]
    fn storage_failure_is_recoverable() {
        let err = SequenceIntegrityError::StorageFailure {
            source: "sqlite".into(),
        };
        assert!(err.can_recover());
    }

    #[spec("SEQ-017")]
    #[test]
    fn ledger_corruption_is_not_recoverable() {
        let err = SequenceIntegrityError::LedgerCorrupted {
            detail: "data loss".into(),
        };
        assert!(!err.can_recover());
    }

    #[spec("SEQ-017")]
    #[test]
    fn display_includes_error_context() {
        let err = SequenceIntegrityError::StorageFailure {
            source: "sqlite".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("sqlite"));
    }

    #[spec("SEQ-017")]
    #[test]
    fn cross_agent_bridge_unavailable_continues_local_tracking() {
        let err = SequenceIntegrityError::TaintBridgeUnavailable;
        assert!(err.can_recover());
    }
}
