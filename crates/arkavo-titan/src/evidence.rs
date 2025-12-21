//! Anomaly evidence types for Titan Monitor
//!
//! Evidence structs capture detected anomalies with minimal allocation overhead.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::Instant;

use torg_core::EvalError;

/// Level of detected anomaly (ordered by severity)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AnomalyLevel {
    /// Level 3: Statistical drift detected
    StatisticalDrift,
    /// Level 2: Input outside known good hypercube
    BoundaryViolation,
    /// Level 1: Hard failure (error, timeout, panic)
    HardFailure,
}

impl AnomalyLevel {
    /// Convert to severity score (0.0 - 1.0)
    #[inline]
    #[must_use]
    pub fn severity(&self) -> f64 {
        match self {
            Self::StatisticalDrift => 0.3,
            Self::BoundaryViolation => 0.7,
            Self::HardFailure => 1.0,
        }
    }
}

/// Details of the detected anomaly
#[derive(Debug, Clone)]
pub enum AnomalyDetails {
    /// Evaluation returned an error
    EvalError {
        /// The error that occurred
        error: EvalError,
    },
    /// Input violated an invariant constraint
    BoundaryViolation {
        /// The contract ID that was violated
        contract_id: uuid::Uuid,
        /// Brief description of the violation
        description: String,
    },
    /// Output distribution drifted from baseline
    Drift {
        /// Which output exhibited drift
        output_id: u16,
        /// Baseline EMA value
        baseline: f64,
        /// Current observed value
        current: f64,
        /// Z-score of the deviation
        z_score: f64,
    },
}

/// Evidence of a detected anomaly
///
/// Designed for minimal allocation: fixed-size fields with optional details.
#[derive(Debug, Clone)]
pub struct AnomalyEvidence {
    /// Severity level of the anomaly
    pub level: AnomalyLevel,
    /// When the anomaly was detected
    pub timestamp: Instant,
    /// Hash of the input that triggered the anomaly
    pub input_hash: u64,
    /// Detailed information about the anomaly
    pub details: AnomalyDetails,
}

impl AnomalyEvidence {
    /// Create evidence for a hard failure
    #[inline]
    pub fn hard_failure(error: EvalError, input_hash: u64) -> Self {
        Self {
            level: AnomalyLevel::HardFailure,
            timestamp: Instant::now(),
            input_hash,
            details: AnomalyDetails::EvalError { error },
        }
    }

    /// Create evidence for a boundary violation
    #[inline]
    pub fn boundary_violation(
        contract_id: uuid::Uuid,
        description: String,
        input_hash: u64,
    ) -> Self {
        Self {
            level: AnomalyLevel::BoundaryViolation,
            timestamp: Instant::now(),
            input_hash,
            details: AnomalyDetails::BoundaryViolation {
                contract_id,
                description,
            },
        }
    }

    /// Create evidence for statistical drift
    #[inline]
    pub fn drift(output_id: u16, baseline: f64, current: f64, z_score: f64, input_hash: u64) -> Self {
        Self {
            level: AnomalyLevel::StatisticalDrift,
            timestamp: Instant::now(),
            input_hash,
            details: AnomalyDetails::Drift {
                output_id,
                baseline,
                current,
                z_score,
            },
        }
    }
}

/// Hash inputs for anomaly tracking
#[inline]
pub fn hash_inputs(inputs: &HashMap<u16, bool>) -> u64 {
    let mut hasher = DefaultHasher::new();
    // Sort keys for deterministic hashing
    let mut pairs: Vec<_> = inputs.iter().collect();
    pairs.sort_by_key(|(k, _)| *k);
    for (k, v) in pairs {
        k.hash(&mut hasher);
        v.hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anomaly_level_ordering() {
        assert!(AnomalyLevel::StatisticalDrift < AnomalyLevel::BoundaryViolation);
        assert!(AnomalyLevel::BoundaryViolation < AnomalyLevel::HardFailure);
    }

    #[test]
    fn test_anomaly_level_severity() {
        assert!(AnomalyLevel::StatisticalDrift.severity() < 0.5);
        assert!(AnomalyLevel::BoundaryViolation.severity() > 0.5);
        assert!((AnomalyLevel::HardFailure.severity() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_hash_inputs_deterministic() {
        let mut inputs1 = HashMap::new();
        inputs1.insert(0, true);
        inputs1.insert(1, false);

        let mut inputs2 = HashMap::new();
        inputs2.insert(1, false);
        inputs2.insert(0, true);

        assert_eq!(hash_inputs(&inputs1), hash_inputs(&inputs2));
    }

    #[test]
    fn test_evidence_creation() {
        let evidence = AnomalyEvidence::hard_failure(EvalError::MissingInput(0), 12345);
        assert_eq!(evidence.level, AnomalyLevel::HardFailure);
        assert_eq!(evidence.input_hash, 12345);
    }
}
