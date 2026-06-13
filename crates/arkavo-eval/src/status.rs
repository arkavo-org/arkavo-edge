//! Terminal eval status and its mapping onto GitHub Check Run conclusions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypedStatus {
    /// Acceptance met.
    Passed,
    /// A metric fell below its threshold.
    RegressionFailed {
        metric: String,
        value: f64,
        threshold: f64,
    },
    /// Pre-flight gate denied (digest mismatch, baseline absent+required, …).
    Refused { reason: String },
    /// First run for this model/prompt-set; nothing to compare against.
    BaselineBootstrapped,
    /// Infrastructure failure (model load, swarm error) — NOT a model regression.
    InfraError { stage: String },
    /// PR did not touch model paths; no check is posted.
    Skipped,
}

impl TypedStatus {
    /// GitHub Check Run `conclusion`, or `None` when no check should be posted.
    pub fn check_conclusion(&self) -> Option<&'static str> {
        match self {
            TypedStatus::Passed => Some("success"),
            TypedStatus::RegressionFailed { .. } => Some("failure"),
            TypedStatus::Refused { .. } => Some("action_required"),
            TypedStatus::BaselineBootstrapped => Some("neutral"),
            TypedStatus::InfraError { .. } => Some("failure"),
            TypedStatus::Skipped => None,
        }
    }

    /// One-line human summary for the check output title.
    pub fn summary(&self) -> String {
        match self {
            TypedStatus::Passed => "Eval passed".into(),
            TypedStatus::RegressionFailed {
                metric,
                value,
                threshold,
            } => {
                format!("Regression: {metric} {value:.4} < {threshold:.4}")
            }
            TypedStatus::Refused { reason } => format!("Refused: {reason}"),
            TypedStatus::BaselineBootstrapped => "Baseline bootstrapped (neutral)".into(),
            TypedStatus::InfraError { stage } => format!("Infrastructure error at {stage}"),
            TypedStatus::Skipped => "Skipped".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conclusions_map_correctly() {
        assert_eq!(TypedStatus::Passed.check_conclusion(), Some("success"));
        assert_eq!(
            TypedStatus::RegressionFailed {
                metric: "similarity".into(),
                value: 0.5,
                threshold: 0.87
            }
            .check_conclusion(),
            Some("failure")
        );
        assert_eq!(
            TypedStatus::Refused { reason: "x".into() }.check_conclusion(),
            Some("action_required")
        );
        assert_eq!(
            TypedStatus::BaselineBootstrapped.check_conclusion(),
            Some("neutral")
        );
        assert_eq!(
            TypedStatus::InfraError {
                stage: "operator".into()
            }
            .check_conclusion(),
            Some("failure")
        );
        assert_eq!(TypedStatus::Skipped.check_conclusion(), None);
    }

    #[test]
    fn infra_error_is_distinct_from_regression() {
        // Both map to "failure" but their summaries must be unambiguous.
        let infra = TypedStatus::InfraError {
            stage: "operator".into(),
        };
        let reg = TypedStatus::RegressionFailed {
            metric: "similarity".into(),
            value: 0.1,
            threshold: 0.87,
        };
        assert!(infra.summary().contains("Infrastructure"));
        assert!(!reg.summary().contains("Infrastructure"));
    }
}
