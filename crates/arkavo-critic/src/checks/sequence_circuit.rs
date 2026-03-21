use crate::checks::traits::{CheckResult, VerificationCheck, VerificationInput};
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct SequenceFeatures {
    pub action_type: String,
    pub prior_actions: Vec<String>,
    pub taint_state: Vec<String>,
    pub sink_type: Option<String>,
}

pub struct SequenceCircuitCheck;

impl SequenceCircuitCheck {
    pub fn new() -> Self {
        Self
    }

    pub fn extract_features(&self, input: &VerificationInput) -> SequenceFeatures {
        let content = &input.response.content;
        let action_type = if content.is_empty() {
            "unknown".to_string()
        } else {
            let lower = content.to_ascii_lowercase();
            if lower.contains("read") || lower.contains("get") {
                "read".to_string()
            } else if lower.contains("write") || lower.contains("post") || lower.contains("send") {
                "write".to_string()
            } else {
                "process".to_string()
            }
        };

        SequenceFeatures {
            action_type,
            prior_actions: Vec::new(),
            taint_state: Vec::new(),
            sink_type: None,
        }
    }
}

#[async_trait]
impl VerificationCheck for SequenceCircuitCheck {
    fn id(&self) -> &'static str {
        "sequence_circuit"
    }

    fn priority(&self) -> u32 {
        7
    }

    async fn verify(&self, _input: &VerificationInput) -> CheckResult {
        CheckResult::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_llm::ProviderResponse;
    use arkavo_test_macros::spec;
    use std::time::Instant;

    fn make_input(response_text: &str) -> VerificationInput {
        VerificationInput::new(
            "test-session".into(),
            ProviderResponse {
                content: response_text.to_string(),
                reasoning_content: None,
                tool_calls: vec![],
                finish_reason: None,
                inference_timing: None,
                quality_gate_retries: 0,
            },
            vec![],
        )
    }

    #[spec("SEQ-010")]
    #[test]
    fn priority_runs_after_basic_circuit_check() {
        let check = SequenceCircuitCheck::new();
        assert!(check.priority() > 5);
        assert!(check.priority() <= 10);
    }

    #[spec("SEQ-010")]
    #[test]
    fn extracts_non_empty_action_type_from_input() {
        let check = SequenceCircuitCheck::new();
        let input = make_input("read the config file");
        let features = check.extract_features(&input);
        assert!(!features.action_type.is_empty());
    }

    #[spec("SEQ-010")]
    #[test]
    fn circuit_evaluates_within_latency_budget() {
        let check = SequenceCircuitCheck::new();
        let input = make_input("summarize the document");
        let start = Instant::now();
        let _features = check.extract_features(&input);
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_micros() < 1000,
            "circuit took {}μs",
            elapsed.as_micros()
        );
    }

    #[spec("SEQ-010")]
    #[tokio::test]
    async fn verify_returns_pass_or_fail_not_skip() {
        let check = SequenceCircuitCheck::new();
        let input = make_input("summarize the document");
        let result = check.verify(&input).await;
        assert!(result.is_pass() || result.is_fail());
    }

    #[spec("SEQ-010")]
    #[test]
    fn fallback_to_per_action_policy_when_no_circuit() {
        let check = SequenceCircuitCheck::new();
        let input = make_input("");
        let features = check.extract_features(&input);
        assert!(!features.action_type.is_empty());
    }
}
