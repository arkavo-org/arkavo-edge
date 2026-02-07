//! Lint check for response quality
//!
//! Validates response quality through pattern matching and heuristics.
//! Fast check (<2ms) that catches common issues.

use super::traits::{CheckResult, VerificationCheck, VerificationInput};
use crate::evidence::{CheckSeverity, VerificationEvidence};
use async_trait::async_trait;
use std::time::Instant;

/// Response quality lint check
///
/// Checks for:
/// - Incomplete responses (truncation)
/// - Repetition patterns
/// - Formatting issues
/// - Common LLM failure modes
pub struct LintCheck {
    priority: u32,
    /// Minimum response length to avoid truncation warning
    min_response_length: usize,
    /// Maximum repetition ratio before warning
    max_repetition_ratio: f64,
}

impl LintCheck {
    /// Create a new lint check with defaults
    pub fn new() -> Self {
        Self {
            priority: 20,
            min_response_length: 10,
            max_repetition_ratio: 0.5,
        }
    }

    /// Create with custom priority
    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }

    /// Set minimum response length
    pub fn with_min_length(mut self, len: usize) -> Self {
        self.min_response_length = len;
        self
    }

    /// Check for truncation indicators
    fn check_truncation(&self, content: &str) -> Option<String> {
        // Check for incomplete sentences/code
        let trimmed = content.trim();

        if trimmed.is_empty() {
            return Some("Empty response".to_string());
        }

        // Check for obvious truncation patterns
        if trimmed.ends_with("...") && trimmed.len() < 50 {
            return Some("Response appears truncated".to_string());
        }

        // Check for unclosed code blocks
        let code_block_starts = content.matches("```").count();
        if !code_block_starts.is_multiple_of(2) {
            return Some("Unclosed code block detected".to_string());
        }

        // Check for unclosed brackets in what looks like JSON
        if content.contains('{') && !content.contains('}') {
            return Some("Unclosed JSON object".to_string());
        }

        None
    }

    /// Check for excessive repetition
    fn check_repetition(&self, content: &str) -> Option<String> {
        let words: Vec<&str> = content.split_whitespace().collect();
        if words.len() < 10 {
            return None;
        }

        // Count unique words
        let unique: std::collections::HashSet<&str> = words.iter().copied().collect();
        let ratio = unique.len() as f64 / words.len() as f64;

        if ratio < self.max_repetition_ratio {
            return Some(format!(
                "High repetition detected ({:.0}% unique words)",
                ratio * 100.0
            ));
        }

        // Check for repeated phrases (3+ word sequences)
        let mut phrase_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        for window in words.windows(3) {
            let phrase = window.join(" ");
            *phrase_counts.entry(phrase).or_insert(0) += 1;
        }

        let repeated_phrases: Vec<_> = phrase_counts
            .iter()
            .filter(|(_, count)| **count > 3)
            .collect();

        if !repeated_phrases.is_empty() {
            return Some(format!(
                "Repeated phrases detected: {} occurrences",
                repeated_phrases.len()
            ));
        }

        None
    }

    /// Check for common LLM failure patterns
    fn check_failure_patterns(&self, content: &str) -> Option<String> {
        let lower = content.to_lowercase();

        // Check for meta-commentary about being an AI
        let meta_patterns = [
            "as an ai language model",
            "as a large language model",
            "i cannot actually",
            "i don't have the ability to",
            "i'm just an ai",
        ];

        for pattern in meta_patterns {
            if lower.contains(pattern) {
                return Some(format!("Meta-commentary detected: contains '{pattern}'"));
            }
        }

        // Check for placeholder text
        let placeholder_patterns = ["[insert", "[todo", "[placeholder", "lorem ipsum"];

        for pattern in placeholder_patterns {
            if lower.contains(pattern) {
                return Some(format!("Placeholder text detected: contains '{pattern}'"));
            }
        }

        None
    }
}

impl Default for LintCheck {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl VerificationCheck for LintCheck {
    fn id(&self) -> &'static str {
        "lint"
    }

    fn priority(&self) -> u32 {
        self.priority
    }

    async fn verify(&self, input: &VerificationInput) -> CheckResult {
        let start = Instant::now();
        let content = &input.response.content;

        // Run all lint checks
        let issues: Vec<String> = [
            self.check_truncation(content),
            self.check_repetition(content),
            self.check_failure_patterns(content),
        ]
        .into_iter()
        .flatten()
        .collect();

        let latency_us = start.elapsed().as_micros() as u64;

        if issues.is_empty() {
            tracing::debug!(check = "lint", latency_us = latency_us, "Lint check passed");
            CheckResult::Pass
        } else if issues.len() == 1 && issues[0].starts_with("Meta-commentary") {
            // Meta-commentary is a warning, not a failure
            let evidence = VerificationEvidence::failed(
                self.id(),
                &issues[0],
                CheckSeverity::Warning,
                latency_us,
            );
            tracing::warn!(
                check = "lint",
                issue = %issues[0],
                latency_us = latency_us,
                "Lint check warning"
            );
            CheckResult::Warn(evidence)
        } else {
            let combined = issues.join("; ");
            let evidence = VerificationEvidence::failed(
                self.id(),
                &combined,
                CheckSeverity::Error,
                latency_us,
            );
            tracing::warn!(
                check = "lint",
                issues = ?issues,
                latency_us = latency_us,
                "Lint check failed"
            );
            CheckResult::Fail(evidence)
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods, clippy::unnecessary_literal_bound)]
mod tests {
    use super::*;
    use arkavo_llm::ProviderResponse;

    fn response(content: &str) -> ProviderResponse {
        ProviderResponse {
            content: content.to_string(),
            reasoning_content: None,
            tool_calls: vec![],
            finish_reason: None,
        }
    }

    #[tokio::test]
    async fn test_lint_pass() {
        let check = LintCheck::new();
        let input = VerificationInput::new(
            "Test".to_string(),
            response("This is a complete and valid response."),
            vec![],
        );

        let result = check.verify(&input).await;
        assert!(result.is_pass());
    }

    #[tokio::test]
    async fn test_lint_empty_response() {
        let check = LintCheck::new();
        let input = VerificationInput::new("Test".to_string(), response(""), vec![]);

        let result = check.verify(&input).await;
        assert!(result.is_fail());
    }

    #[tokio::test]
    async fn test_lint_unclosed_code_block() {
        let check = LintCheck::new();
        let input = VerificationInput::new(
            "Test".to_string(),
            response("Here's some code:\n```python\nprint('hello')"),
            vec![],
        );

        let result = check.verify(&input).await;
        assert!(result.is_fail());
        assert!(
            result
                .evidence()
                .unwrap()
                .description
                .contains("code block")
        );
    }

    #[tokio::test]
    async fn test_lint_repetition() {
        let check = LintCheck::new();
        let repeated = "the the the the the the the the the the the the";
        let input = VerificationInput::new("Test".to_string(), response(repeated), vec![]);

        let result = check.verify(&input).await;
        assert!(result.is_fail());
        assert!(
            result
                .evidence()
                .unwrap()
                .description
                .contains("repetition")
        );
    }

    #[tokio::test]
    async fn test_lint_ai_meta_commentary() {
        let check = LintCheck::new();
        let input = VerificationInput::new(
            "Test".to_string(),
            response("As an AI language model, I cannot help with that."),
            vec![],
        );

        let result = check.verify(&input).await;
        // Meta-commentary is a warning, not a failure
        assert!(matches!(result, CheckResult::Warn(_)));
    }

    #[tokio::test]
    async fn test_lint_placeholder() {
        let check = LintCheck::new();
        let input = VerificationInput::new(
            "Test".to_string(),
            response("Here is the code: [TODO: implement this]"),
            vec![],
        );

        let result = check.verify(&input).await;
        assert!(result.is_fail());
        assert!(
            result
                .evidence()
                .unwrap()
                .description
                .contains("Placeholder")
        );
    }
}
