use crate::checks::traits::{CheckResult, VerificationCheck, VerificationInput};
use async_trait::async_trait;
use std::time::Duration;

#[derive(Debug, PartialEq)]
pub enum ConsequenceLevel {
    Low,
    High,
}

const HIGH_CONSEQUENCE_ACTIONS: &[&str] = &[
    "http_post",
    "http_put",
    "http_delete",
    "delete_file",
    "delete_dir",
    "remove",
    "use_api_key",
    "use_credential",
    "send_email",
    "deploy",
    "execute",
    "shell",
];

pub struct SequenceGate {
    _latency_budget: Duration,
}

impl SequenceGate {
    pub fn new(latency_budget: Duration) -> Self {
        Self {
            _latency_budget: latency_budget,
        }
    }

    pub fn classify_action(&self, tool_name: &str) -> ConsequenceLevel {
        let lower = tool_name.to_ascii_lowercase();
        for action in HIGH_CONSEQUENCE_ACTIONS {
            if lower.contains(action) {
                return ConsequenceLevel::High;
            }
        }
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
        CheckResult::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;
    use std::time::Instant;

    #[spec("SEQ-011")]
    #[test]
    fn classifies_egress_as_high_consequence() {
        let gate = SequenceGate::new(Duration::from_micros(100));
        assert_eq!(gate.classify_action("http_post"), ConsequenceLevel::High);
    }

    #[spec("SEQ-011")]
    #[test]
    fn classifies_delete_as_high_consequence() {
        let gate = SequenceGate::new(Duration::from_micros(100));
        assert_eq!(gate.classify_action("delete_file"), ConsequenceLevel::High);
    }

    #[spec("SEQ-011")]
    #[test]
    fn classifies_read_as_low_consequence() {
        let gate = SequenceGate::new(Duration::from_micros(100));
        assert_eq!(gate.classify_action("read_file"), ConsequenceLevel::Low);
    }

    #[spec("SEQ-011")]
    #[test]
    fn classifies_credential_use_as_high_consequence() {
        let gate = SequenceGate::new(Duration::from_micros(100));
        assert_eq!(gate.classify_action("use_api_key"), ConsequenceLevel::High);
    }

    #[spec("SEQ-011")]
    #[test]
    fn classification_completes_within_latency_budget() {
        let gate = SequenceGate::new(Duration::from_micros(100));
        let start = Instant::now();
        let _level = gate.classify_action("http_post");
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_micros() < 100,
            "gate took {}μs",
            elapsed.as_micros()
        );
    }

    #[spec("SEQ-011")]
    #[test]
    fn first_action_without_context_uses_per_action_policy() {
        let gate = SequenceGate::new(Duration::from_micros(100));
        assert_eq!(gate.classify_action("read_file"), ConsequenceLevel::Low);
    }
}
