//! Verification evidence types

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Severity level for verification findings
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum CheckSeverity {
    /// Informational - does not affect pass/fail
    Info = 1,
    /// Warning - may indicate issues
    Warning = 2,
    /// Error - verification failed
    Error = 3,
    /// Critical - security or safety concern
    Critical = 4,
}

impl CheckSeverity {
    /// Check if this severity should cause failure at given threshold
    pub fn should_fail(&self, threshold: u8) -> bool {
        (*self as u8) >= threshold
    }
}

/// Status of the verification process
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerificationStatus {
    /// Verification passed
    Passed,
    /// Verification failed with reason
    Failed { reason: String },
    /// Verification partially passed with score
    Partial { score: f64, notes: String },
    /// Awaiting human approval
    Pending { task_id: Uuid },
    /// Skipped (check not applicable)
    Skipped { reason: String },
}

impl VerificationStatus {
    /// Check if status is a pass
    pub fn is_passed(&self) -> bool {
        matches!(self, Self::Passed)
    }

    /// Check if status is a failure
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    /// Check if status is pending human approval
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending { .. })
    }
}

/// Evidence from a verification check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationEvidence {
    /// Unique identifier for this evidence
    pub id: Uuid,
    /// Which check produced this evidence
    pub check_id: String,
    /// When the check was performed
    pub timestamp: DateTime<Utc>,
    /// Status of the verification
    pub status: VerificationStatus,
    /// Severity of any issues found
    pub severity: CheckSeverity,
    /// Human-readable description
    pub description: String,
    /// Additional context/details
    pub details: Option<serde_json::Value>,
    /// Time taken for this check (microseconds)
    pub latency_us: u64,
}

impl VerificationEvidence {
    /// Create a passing evidence
    pub fn passed(check_id: &str, description: &str, latency_us: u64) -> Self {
        Self {
            id: Uuid::new_v4(),
            check_id: check_id.to_string(),
            timestamp: Utc::now(),
            status: VerificationStatus::Passed,
            severity: CheckSeverity::Info,
            description: description.to_string(),
            details: None,
            latency_us,
        }
    }

    /// Create a failing evidence
    pub fn failed(check_id: &str, reason: &str, severity: CheckSeverity, latency_us: u64) -> Self {
        Self {
            id: Uuid::new_v4(),
            check_id: check_id.to_string(),
            timestamp: Utc::now(),
            status: VerificationStatus::Failed {
                reason: reason.to_string(),
            },
            severity,
            description: reason.to_string(),
            details: None,
            latency_us,
        }
    }

    /// Create a partial pass evidence
    pub fn partial(check_id: &str, score: f64, notes: &str, latency_us: u64) -> Self {
        Self {
            id: Uuid::new_v4(),
            check_id: check_id.to_string(),
            timestamp: Utc::now(),
            status: VerificationStatus::Partial {
                score,
                notes: notes.to_string(),
            },
            severity: if score < 0.5 {
                CheckSeverity::Warning
            } else {
                CheckSeverity::Info
            },
            description: notes.to_string(),
            details: None,
            latency_us,
        }
    }

    /// Create a pending (HITL) evidence
    pub fn pending(check_id: &str, task_id: Uuid, reason: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            check_id: check_id.to_string(),
            timestamp: Utc::now(),
            status: VerificationStatus::Pending { task_id },
            severity: CheckSeverity::Warning,
            description: reason.to_string(),
            details: None,
            latency_us: 0,
        }
    }

    /// Create a skipped evidence
    pub fn skipped(check_id: &str, reason: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            check_id: check_id.to_string(),
            timestamp: Utc::now(),
            status: VerificationStatus::Skipped {
                reason: reason.to_string(),
            },
            severity: CheckSeverity::Info,
            description: reason.to_string(),
            details: None,
            latency_us: 0,
        }
    }

    /// Add details to the evidence
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[test]
    fn test_severity_ordering() {
        assert!(CheckSeverity::Info < CheckSeverity::Warning);
        assert!(CheckSeverity::Warning < CheckSeverity::Error);
        assert!(CheckSeverity::Error < CheckSeverity::Critical);
    }

    #[test]
    fn test_severity_should_fail() {
        assert!(!CheckSeverity::Info.should_fail(2));
        assert!(CheckSeverity::Warning.should_fail(2));
        assert!(CheckSeverity::Error.should_fail(2));
        assert!(CheckSeverity::Critical.should_fail(2));
    }

    #[test]
    fn test_verification_status() {
        assert!(VerificationStatus::Passed.is_passed());
        assert!(!VerificationStatus::Passed.is_failed());

        let failed = VerificationStatus::Failed {
            reason: "test".to_string(),
        };
        assert!(failed.is_failed());
        assert!(!failed.is_passed());
    }

    #[spec("CRIT-006")]
    #[test]
    fn test_evidence_creation() {
        let evidence = VerificationEvidence::passed("schema", "All schemas valid", 1);
        assert!(evidence.status.is_passed());
        assert_eq!(evidence.latency_us, 1);

        let evidence = VerificationEvidence::failed("policy", "Violation", CheckSeverity::Error, 2);
        assert!(evidence.status.is_failed());
        assert_eq!(evidence.severity, CheckSeverity::Error);
    }
}
