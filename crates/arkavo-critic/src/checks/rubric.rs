//! Rubric check for graded evaluation scoring
//!
//! Runs after safety checks (priority 200) to produce a four-dimension
//! score that feeds Thompson Sampling with continuous signal.
//! Only applicable when the verification input contains acceptance criteria
//! (i.e., a task contract is active).

use super::traits::{CheckResult, VerificationCheck, VerificationInput};
use crate::evidence::VerificationEvidence;
use crate::rubric::{EvaluationRubric, RubricWeights};
use async_trait::async_trait;
use std::time::Instant;

/// Graded evaluation check using the four-dimension rubric.
///
/// Produces continuous scores rather than binary pass/fail.
/// Only runs when acceptance criteria are present in the input context.
pub struct RubricCheck {
    priority: u32,
    weights: RubricWeights,
}

impl RubricCheck {
    pub fn new() -> Self {
        Self {
            priority: 200,
            weights: RubricWeights::default(),
        }
    }

    pub fn with_weights(mut self, weights: RubricWeights) -> Self {
        self.weights = weights;
        self
    }

    /// Score goal fidelity: how well does the response match acceptance criteria?
    fn score_goal_fidelity(&self, input: &VerificationInput) -> f64 {
        let criteria = match self.extract_criteria(input) {
            Some(c) => c,
            None => return 0.5,
        };
        if criteria.is_empty() {
            return 0.5;
        }

        let response_lower = input.response.content.to_lowercase();
        let matched = criteria
            .iter()
            .filter(|criterion| {
                let criterion_lower = criterion.to_lowercase();
                // Extract key terms from criterion (words > 3 chars)
                let key_terms: Vec<&str> = criterion_lower
                    .split_whitespace()
                    .filter(|w| w.len() > 3)
                    .collect();
                if key_terms.is_empty() {
                    return true;
                }
                // At least half the key terms should appear in response
                let hits = key_terms
                    .iter()
                    .filter(|t| response_lower.contains(**t))
                    .count();
                hits * 2 >= key_terms.len()
            })
            .count();

        matched as f64 / criteria.len() as f64
    }

    /// Score efficiency: was the response concise relative to the task?
    fn score_efficiency(&self, input: &VerificationInput) -> f64 {
        let content_len = input.response.content.len();

        // Check for token budget in context
        if let Some(ctx) = &input.context
            && let Some(max_tokens) = ctx.get("max_tokens").and_then(|v| v.as_u64())
        {
            // Approximate: 1 token ~ 4 chars
            let estimated_tokens = content_len as u64 / 4;
            if max_tokens > 0 {
                let usage_ratio = estimated_tokens as f64 / max_tokens as f64;
                // Penalize both under-use (<10% suggests incomplete) and over-use
                return if usage_ratio < 0.1 {
                    0.3
                } else if usage_ratio <= 0.8 {
                    1.0
                } else {
                    (1.0 - (usage_ratio - 0.8) / 0.4).clamp(0.3, 1.0)
                };
            }
        }

        // Without budget info, score based on response being non-trivial
        if content_len == 0 {
            0.0
        } else if content_len < 20 {
            0.4
        } else {
            0.7
        }
    }

    /// Score state coherence: is the response well-formed and consistent?
    fn score_state_coherence(&self, input: &VerificationInput) -> f64 {
        let content = &input.response.content;
        let mut score: f64 = 1.0;

        // Check JSON validity if response looks like JSON
        let trimmed = content.trim();
        if (trimmed.starts_with('{') || trimmed.starts_with('['))
            && serde_json::from_str::<serde_json::Value>(trimmed).is_err()
        {
            score -= 0.4;
        }

        // Check for unclosed code blocks
        let fence_count = content.matches("```").count();
        if !fence_count.is_multiple_of(2) {
            score -= 0.2;
        }

        // Check for contradiction indicators
        let contradiction_phrases = [
            "but actually",
            "wait, no",
            "I was wrong",
            "let me correct",
            "actually, ignore",
        ];
        for phrase in &contradiction_phrases {
            if content.to_lowercase().contains(phrase) {
                score -= 0.15;
            }
        }

        score.clamp(0.0, 1.0)
    }

    /// Score adaptability: did the response handle unexpected conditions?
    fn score_adaptability(&self, input: &VerificationInput) -> f64 {
        let content = &input.response.content;
        let mut score: f64 = 0.5; // neutral baseline

        // Positive: response acknowledges limitations or provides alternatives
        let adaptive_phrases = [
            "alternatively",
            "fallback",
            "if that fails",
            "as a workaround",
            "in case",
            "however",
        ];
        for phrase in &adaptive_phrases {
            if content.to_lowercase().contains(phrase) {
                score += 0.1;
            }
        }

        // Positive: tool calls present (indicates acting on environment)
        if !input.response.tool_calls.is_empty() {
            score += 0.15;
        }

        // Negative: response is a bare refusal without alternative
        let refusal_phrases = ["i cannot", "i can't", "unable to", "not possible"];
        let has_refusal = refusal_phrases
            .iter()
            .any(|p| content.to_lowercase().contains(p));
        let has_alternative = adaptive_phrases
            .iter()
            .any(|p| content.to_lowercase().contains(p));
        if has_refusal && !has_alternative {
            score -= 0.3;
        }

        score.clamp(0.0, 1.0)
    }

    /// Extract acceptance criteria strings from input context
    fn extract_criteria(&self, input: &VerificationInput) -> Option<Vec<String>> {
        let ctx = input.context.as_ref()?;
        let criteria = ctx.get("acceptance_criteria")?;
        let arr = criteria.as_array()?;
        Some(
            arr.iter()
                .filter_map(|v| {
                    // Support both string and object with "criterion" field
                    v.as_str().map(String::from).or_else(|| {
                        v.get("criterion")
                            .and_then(|c| c.as_str())
                            .map(String::from)
                    })
                })
                .collect(),
        )
    }
}

impl Default for RubricCheck {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl VerificationCheck for RubricCheck {
    fn id(&self) -> &'static str {
        "rubric"
    }

    fn priority(&self) -> u32 {
        self.priority
    }

    fn is_applicable(&self, input: &VerificationInput) -> bool {
        input
            .context
            .as_ref()
            .and_then(|ctx| ctx.get("acceptance_criteria"))
            .is_some()
    }

    async fn verify(&self, input: &VerificationInput) -> CheckResult {
        let start = Instant::now();

        let rubric = EvaluationRubric::new(
            self.score_goal_fidelity(input),
            self.score_efficiency(input),
            self.score_state_coherence(input),
            self.score_adaptability(input),
        );

        let composite = rubric.composite(&self.weights);
        let elapsed_us = start.elapsed().as_micros() as u64;

        let details = serde_json::json!({
            "goal_fidelity": rubric.goal_fidelity,
            "efficiency": rubric.efficiency,
            "state_coherence": rubric.state_coherence,
            "adaptability": rubric.adaptability,
            "composite": composite,
            "weights": {
                "goal_fidelity": self.weights.goal_fidelity,
                "efficiency": self.weights.efficiency,
                "state_coherence": self.weights.state_coherence,
                "adaptability": self.weights.adaptability,
            }
        });

        let notes = format!(
            "gf={:.2} eff={:.2} sc={:.2} ad={:.2} => {:.2}",
            rubric.goal_fidelity,
            rubric.efficiency,
            rubric.state_coherence,
            rubric.adaptability,
            composite
        );

        let evidence = VerificationEvidence::partial("rubric", composite, &notes, elapsed_us)
            .with_details(details);

        // Rubric never fails the pipeline — it provides graded signal
        CheckResult::Warn(evidence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_llm::ProviderResponse;

    fn make_input(content: &str, context: Option<serde_json::Value>) -> VerificationInput {
        let response = ProviderResponse {
            content: content.to_string(),
            reasoning_content: None,
            tool_calls: vec![],
            finish_reason: None,
            inference_timing: None,
            quality_gate_retries: 0,
        };
        let mut input = VerificationInput::new("test task".to_string(), response, vec![]);
        if let Some(ctx) = context {
            input = input.with_context(ctx);
        }
        input
    }

    #[test]
    fn test_not_applicable_without_criteria() {
        let check = RubricCheck::new();
        let input = make_input("hello", None);
        assert!(!check.is_applicable(&input));
    }

    #[test]
    fn test_applicable_with_criteria() {
        let check = RubricCheck::new();
        let ctx = serde_json::json!({
            "acceptance_criteria": ["zone covers minimum area"]
        });
        let input = make_input("hello", Some(ctx));
        assert!(check.is_applicable(&input));
    }

    #[tokio::test]
    async fn test_verify_produces_graded_evidence() {
        let check = RubricCheck::new();
        let ctx = serde_json::json!({
            "acceptance_criteria": ["response mentions zone placement"],
            "max_tokens": 1024
        });
        let input = make_input(
            "The zone placement covers the required area with fertile soil.",
            Some(ctx),
        );
        let result = check.verify(&input).await;
        assert!(matches!(result, CheckResult::Warn(_)));
        if let CheckResult::Warn(evidence) = result {
            if let crate::VerificationStatus::Partial { score, .. } = &evidence.status {
                assert!(*score > 0.0);
                assert!(*score <= 1.0);
            } else {
                panic!("expected Partial status");
            }
            assert!(evidence.details.is_some());
        }
    }

    #[test]
    fn test_goal_fidelity_all_criteria_met() {
        let check = RubricCheck::new();
        let ctx = serde_json::json!({
            "acceptance_criteria": ["covers fertile area", "no overlap with stockpile"]
        });
        let input = make_input(
            "The zone covers a large fertile area and has no overlap with the stockpile zones.",
            Some(ctx),
        );
        let score = check.score_goal_fidelity(&input);
        assert!(score > 0.5, "score was {score}");
    }

    #[test]
    fn test_goal_fidelity_no_criteria_met() {
        let check = RubricCheck::new();
        let ctx = serde_json::json!({
            "acceptance_criteria": ["quantum entanglement verified"]
        });
        let input = make_input("The zone covers fertile soil.", Some(ctx));
        let score = check.score_goal_fidelity(&input);
        assert!(score < 0.5, "score was {score}");
    }

    #[test]
    fn test_efficiency_within_budget() {
        let check = RubricCheck::new();
        let ctx = serde_json::json!({
            "acceptance_criteria": ["test"],
            "max_tokens": 1024
        });
        // ~50 chars = ~12 tokens, well within 1024
        let input = make_input(
            "This is a concise and on-topic response to the task.",
            Some(ctx),
        );
        let score = check.score_efficiency(&input);
        assert!(score >= 0.3, "score was {score}");
    }

    #[test]
    fn test_state_coherence_valid_json() {
        let check = RubricCheck::new();
        let ctx = serde_json::json!({ "acceptance_criteria": ["valid"] });
        let input = make_input("{\"zone\": \"placed\", \"area\": 36}", Some(ctx));
        let score = check.score_state_coherence(&input);
        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_state_coherence_invalid_json() {
        let check = RubricCheck::new();
        let ctx = serde_json::json!({ "acceptance_criteria": ["valid"] });
        let input = make_input("{\"zone\": \"placed\", broken", Some(ctx));
        let score = check.score_state_coherence(&input);
        assert!(score < 1.0);
    }

    #[test]
    fn test_adaptability_with_alternatives() {
        let check = RubricCheck::new();
        let ctx = serde_json::json!({ "acceptance_criteria": ["test"] });
        let input = make_input(
            "The primary approach works. Alternatively, if that fails, we can use a fallback strategy.",
            Some(ctx),
        );
        let score = check.score_adaptability(&input);
        assert!(score > 0.5, "score was {score}");
    }

    #[test]
    fn test_adaptability_bare_refusal() {
        let check = RubricCheck::new();
        let ctx = serde_json::json!({ "acceptance_criteria": ["test"] });
        let input = make_input("I cannot complete this task.", Some(ctx));
        let score = check.score_adaptability(&input);
        assert!(score < 0.5, "score was {score}");
    }

    #[test]
    fn test_extract_criteria_strings() {
        let check = RubricCheck::new();
        let ctx = serde_json::json!({
            "acceptance_criteria": ["criterion one", "criterion two"]
        });
        let input = make_input("test", Some(ctx));
        let criteria = check.extract_criteria(&input).unwrap();
        assert_eq!(criteria.len(), 2);
    }

    #[test]
    fn test_extract_criteria_objects() {
        let check = RubricCheck::new();
        let ctx = serde_json::json!({
            "acceptance_criteria": [
                {"id": "ac1", "criterion": "zone covers area"},
                {"id": "ac2", "criterion": "no overlap"}
            ]
        });
        let input = make_input("test", Some(ctx));
        let criteria = check.extract_criteria(&input).unwrap();
        assert_eq!(criteria.len(), 2);
        assert_eq!(criteria[0], "zone covers area");
    }
}
