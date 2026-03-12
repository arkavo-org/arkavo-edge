//! Quality evaluation for autoresearch experiments
//!
//! Inline critic checks without full CriticPipeline dependency.
//! Checks: non-empty, truncation, repetition, placeholder, AI meta patterns.

use std::collections::HashSet;

/// Evaluate response quality, returning a score in [0.0, 1.0]
///
/// Deductions:
/// - Empty response: 0.0
/// - Unclosed code blocks (truncation): -0.3
/// - Severe repetition (unique ratio < 0.3): -0.5
/// - Moderate repetition (unique ratio < 0.5): -0.2
/// - Failure/placeholder patterns: -0.3 each
/// - Very short response (< 3 words): -0.2
pub fn evaluate_quality(response: &str) -> f64 {
    let trimmed = response.trim();

    if trimmed.is_empty() {
        return 0.0;
    }

    let mut score: f64 = 1.0;

    // Truncation: unclosed code blocks
    let backtick_count = trimmed.matches("```").count();
    if !backtick_count.is_multiple_of(2) {
        score -= 0.3;
    }

    // Repetition: unique word ratio
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.len() >= 10 {
        let unique: HashSet<&str> = words.iter().copied().collect();
        let ratio = unique.len() as f64 / words.len() as f64;
        if ratio < 0.3 {
            score -= 0.5;
        } else if ratio < 0.5 {
            score -= 0.2;
        }
    }

    // Placeholder / failure patterns
    let lower = trimmed.to_lowercase();
    for pattern in FAILURE_PATTERNS {
        if lower.contains(pattern) {
            score -= 0.3;
        }
    }

    // Very short response
    if words.len() < 3 {
        score -= 0.2;
    }

    score.clamp(0.0, 1.0)
}

const FAILURE_PATTERNS: &[&str] = &[
    "as an ai language model",
    "as a large language model",
    "[todo",
    "[placeholder",
    "lorem ipsum",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_good_response() {
        let score = evaluate_quality("The capital of France is Paris.");
        assert!(score > 0.8, "score={score}");
    }

    #[test]
    fn test_empty_response() {
        let score = evaluate_quality("");
        assert!(score < f64::EPSILON);
    }

    #[test]
    fn test_repetitive_response() {
        let repeated = "the the the the the the the the the the the the the the the";
        let score = evaluate_quality(repeated);
        assert!(score <= 0.5, "score={score}");
    }

    #[test]
    fn test_truncated_code() {
        let truncated = "Here's the code:\n```python\ndef foo():";
        let score = evaluate_quality(truncated);
        assert!(score < 0.8, "score={score}");
    }

    #[test]
    fn test_placeholder() {
        let placeholder = "Here is the answer: [TODO: implement this]";
        let score = evaluate_quality(placeholder);
        assert!(score < 0.8, "score={score}");
    }

    #[test]
    fn test_ai_meta() {
        let meta = "As an AI language model, I cannot help with that request.";
        let score = evaluate_quality(meta);
        assert!(score < 0.8, "score={score}");
    }

    #[test]
    fn test_whitespace_only() {
        let score = evaluate_quality("   \n\t  ");
        assert!(score < f64::EPSILON);
    }

    #[test]
    fn test_closed_code_blocks() {
        let code = "Here:\n```python\ndef foo(): pass\n```\nDone.";
        let score = evaluate_quality(code);
        assert!(score > 0.8, "score={score}");
    }
}
