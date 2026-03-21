use crate::checks::traits::{CheckResult, VerificationCheck, VerificationInput};
use async_trait::async_trait;

/// Sequence context features for circuit evaluation
#[derive(Debug, Clone)]
pub struct SequenceFeatures {
    pub action_type: String,
    pub prior_actions: Vec<String>,
    pub taint_state: Vec<String>,
    pub sink_type: Option<String>,
}

/// SEQ-010: Sequence-aware CircuitCheck that evaluates TØR-G circuits
/// with sequence context features
pub struct SequenceCircuitCheck;

impl SequenceCircuitCheck {
    pub fn new() -> Self {
        Self
    }

    /// Extract sequence features from input context
    pub fn extract_features(&self, _input: &VerificationInput) -> SequenceFeatures {
        SequenceFeatures {
            action_type: String::new(),
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
        100
    }

    async fn verify(&self, _input: &VerificationInput) -> CheckResult {
        CheckResult::Skip("not implemented".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_llm::ProviderResponse;

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

    // =========================================================================
    // SEQ-010: Evaluate sequence-aware TØR-G circuit before action
    // =========================================================================

    #[test]
    fn priority_lower_than_basic_circuit_check() {
        let check = SequenceCircuitCheck::new();
        // SequenceCircuitCheck should run after basic CircuitCheck (priority 5)
        assert!(check.priority() > 5);
        assert!(check.priority() <= 10);
    }

    #[test]
    fn extracts_sequence_features_from_input() {
        let check = SequenceCircuitCheck::new();
        let input = make_input("read the config file");
        let features = check.extract_features(&input);
        assert!(!features.action_type.is_empty());
    }

    #[tokio::test]
    async fn pass_allows_action_to_proceed() {
        let check = SequenceCircuitCheck::new();
        let input = make_input("summarize the document");
        let result = check.verify(&input).await;
        assert!(result.is_pass());
    }
}
