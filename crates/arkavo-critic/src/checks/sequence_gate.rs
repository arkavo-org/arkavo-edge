use crate::checks::traits::{CheckResult, VerificationCheck, VerificationInput};
use async_trait::async_trait;
use std::time::Duration;

/// Classification of action consequence level
#[derive(Debug, PartialEq)]
pub enum ConsequenceLevel {
    Low,
    High,
}

/// SEQ-011: Synchronous gate for high-consequence actions
pub struct SequenceGate {
    _latency_budget: Duration,
}

impl SequenceGate {
    pub fn new(latency_budget: Duration) -> Self {
        Self {
            _latency_budget: latency_budget,
        }
    }

    /// Classify action consequence level
    pub fn classify_action(&self, _tool_name: &str) -> ConsequenceLevel {
        ConsequenceLevel::Low
    }
}

#[async_trait]
impl VerificationCheck for SequenceGate {
    fn id(&self) -> &'static str {
        "sequence_gate"
    }

    fn priority(&self) -> u32 {
        6
    }

    async fn verify(&self, _input: &VerificationInput) -> CheckResult {
        CheckResult::Skip("not implemented".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // SEQ-011: Synchronous gate on high-consequence actions
    // =========================================================================

    #[test]
    fn egress_classified_as_high_consequence() {
        let gate = SequenceGate::new(Duration::from_micros(100));
        assert_eq!(
            gate.classify_action("http_post"),
            ConsequenceLevel::High,
        );
    }

    #[test]
    fn delete_classified_as_high_consequence() {
        let gate = SequenceGate::new(Duration::from_micros(100));
        assert_eq!(
            gate.classify_action("delete_file"),
            ConsequenceLevel::High,
        );
    }

    #[test]
    fn read_classified_as_low_consequence() {
        let gate = SequenceGate::new(Duration::from_micros(100));
        assert_eq!(
            gate.classify_action("read_file"),
            ConsequenceLevel::Low,
        );
    }

    #[test]
    fn credential_use_classified_as_high_consequence() {
        let gate = SequenceGate::new(Duration::from_micros(100));
        assert_eq!(
            gate.classify_action("use_api_key"),
            ConsequenceLevel::High,
        );
    }

    #[test]
    fn gate_has_100_microsecond_budget() {
        let gate = SequenceGate::new(Duration::from_micros(100));
        assert_eq!(gate.classify_action("read_file"), ConsequenceLevel::Low);
    }
}
