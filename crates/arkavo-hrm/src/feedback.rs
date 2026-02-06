//! Feedback types for model learning and pattern detection

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Type of feedback issue detected in model output
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum FeedbackIssue {
    /// Model wrapped output in code fences when not needed
    UnwantedCodeFence,
    /// Model called a tool that doesn't exist
    HallucinatedTool,
    /// Model used wrong tool call format
    InvalidToolFormat,
    /// Model refused to answer when it should have
    UnexpectedRefusal,
    /// Model looped or repeated output
    OutputLoop,
    /// Model triggered wrong MoE expert (e.g., coding for chat)
    WrongExpertRouting,
    /// Model output was empty or timed out
    EmptyOrTimeout,
    /// Response was correct
    #[default]
    Correct,
}

/// Feedback record for model learning
/// Captures what went wrong and how to improve
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BurstFeedback {
    /// Unique feedback ID
    pub id: Uuid,
    /// When this feedback was recorded
    pub timestamp: DateTime<Utc>,
    /// Model that generated the response
    pub model_name: String,
    /// Model family (glm, gemma, qwen, etc.)
    pub model_family: String,
    /// Original user prompt
    pub prompt: String,
    /// Keywords extracted from prompt
    pub prompt_keywords: Vec<String>,
    /// Model's raw response
    pub response: String,
    /// Type of issue detected
    pub issue: FeedbackIssue,
    /// Human-readable description of the issue
    pub issue_description: String,
    /// What the correct behavior should have been
    pub expected_behavior: Option<String>,
    /// Confidence in this feedback (0.0-1.0)
    pub confidence: f64,
    /// Associated task/subtask IDs
    pub task_id: Option<Uuid>,
    pub subtask_id: Option<Uuid>,
}

impl BurstFeedback {
    /// Create new feedback record
    pub fn new(model_name: String, model_family: String, prompt: String, response: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            model_name,
            model_family,
            prompt,
            prompt_keywords: Vec::new(),
            response,
            issue: FeedbackIssue::Correct,
            issue_description: String::new(),
            expected_behavior: None,
            confidence: 1.0,
            task_id: None,
            subtask_id: None,
        }
    }

    /// Record a code fence issue
    pub fn with_code_fence_issue(mut self) -> Self {
        self.issue = FeedbackIssue::UnwantedCodeFence;
        self.issue_description = "Model wrapped response in code fences".to_string();
        self.expected_behavior = Some("Plain text response without formatting".to_string());
        self
    }

    /// Record an output loop/timeout issue
    pub fn with_loop_issue(mut self) -> Self {
        self.issue = FeedbackIssue::OutputLoop;
        self.issue_description = "Model output looped or timed out".to_string();
        self
    }

    /// Record wrong expert routing (MoE models)
    pub fn with_wrong_expert(mut self, expected_expert: &str) -> Self {
        self.issue = FeedbackIssue::WrongExpertRouting;
        self.issue_description = format!(
            "Expected {} expert, got coding/tool expert",
            expected_expert
        );
        self
    }

    /// Extract keywords from the prompt for pattern matching
    pub fn extract_keywords(&mut self) {
        let lower = self.prompt.to_lowercase();
        let mut keywords = Vec::new();

        // Math indicators
        if lower.contains('+') || lower.contains('-') || lower.contains('*') || lower.contains('/')
        {
            keywords.push("math_operator".to_string());
        }
        if lower.contains("calculate") || lower.contains("compute") || lower.contains("sum") {
            keywords.push("math_verb".to_string());
        }

        // Question indicators
        if lower.contains("what is") || lower.contains("what's") {
            keywords.push("what_question".to_string());
        }
        if lower.contains("how") {
            keywords.push("how_question".to_string());
        }

        // Code indicators
        if lower.contains("code") || lower.contains("function") || lower.contains("program") {
            keywords.push("code_request".to_string());
        }

        // Greeting indicators
        if lower.starts_with("hi") || lower.starts_with("hello") || lower.starts_with("hey") {
            keywords.push("greeting".to_string());
        }

        self.prompt_keywords = keywords;
    }
}

/// Learned pattern from feedback
/// Used to adjust prompts for specific models
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPattern {
    /// Pattern ID
    pub id: Uuid,
    /// Model family this pattern applies to
    pub model_family: String,
    /// Keywords that trigger this pattern
    pub trigger_keywords: Vec<String>,
    /// The issue this pattern addresses
    pub issue_type: FeedbackIssue,
    /// Recommended prompt adjustment
    pub prompt_adjustment: String,
    /// How many times this pattern was observed
    pub observation_count: u32,
    /// Success rate after applying adjustment (0.0-1.0)
    pub success_rate: f64,
    /// When pattern was first observed
    pub first_seen: DateTime<Utc>,
    /// When pattern was last observed
    pub last_seen: DateTime<Utc>,
}

impl ModelPattern {
    /// Create a new pattern from feedback
    pub fn from_feedback(feedback: &BurstFeedback) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            model_family: feedback.model_family.clone(),
            trigger_keywords: feedback.prompt_keywords.clone(),
            issue_type: feedback.issue,
            prompt_adjustment: String::new(),
            observation_count: 1,
            success_rate: 0.0,
            first_seen: now,
            last_seen: now,
        }
    }

    /// Update pattern with new observation
    pub fn observe(&mut self) {
        self.observation_count += 1;
        self.last_seen = Utc::now();
    }

    /// Record a success/failure after applying adjustment
    pub fn record_outcome(&mut self, success: bool) {
        let alpha = 0.2;
        let outcome = if success { 1.0 } else { 0.0 };
        self.success_rate = alpha * outcome + (1.0 - alpha) * self.success_rate;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feedback_issue_default() {
        assert_eq!(FeedbackIssue::default(), FeedbackIssue::Correct);
    }

    #[test]
    fn test_burst_feedback_new_defaults() {
        let f = BurstFeedback::new("m".into(), "fam".into(), "p".into(), "r".into());
        assert_eq!(f.issue, FeedbackIssue::Correct);
        assert_eq!(f.confidence, 1.0);
        assert!(f.prompt_keywords.is_empty());
        assert!(f.expected_behavior.is_none());
    }

    #[test]
    fn test_with_code_fence_issue() {
        let f = BurstFeedback::new("m".into(), "f".into(), "p".into(), "r".into())
            .with_code_fence_issue();
        assert_eq!(f.issue, FeedbackIssue::UnwantedCodeFence);
        assert!(!f.issue_description.is_empty());
        assert!(f.expected_behavior.is_some());
    }

    #[test]
    fn test_with_loop_issue() {
        let f =
            BurstFeedback::new("m".into(), "f".into(), "p".into(), "r".into()).with_loop_issue();
        assert_eq!(f.issue, FeedbackIssue::OutputLoop);
    }

    #[test]
    fn test_with_wrong_expert() {
        let f = BurstFeedback::new("m".into(), "f".into(), "p".into(), "r".into())
            .with_wrong_expert("conversation");
        assert_eq!(f.issue, FeedbackIssue::WrongExpertRouting);
        assert!(f.issue_description.contains("conversation"));
    }

    #[test]
    fn test_extract_keywords_math() {
        let mut f = BurstFeedback::new("m".into(), "f".into(), "what is 2+3".into(), "5".into());
        f.extract_keywords();
        assert!(f.prompt_keywords.contains(&"math_operator".to_string()));
        assert!(f.prompt_keywords.contains(&"what_question".to_string()));
    }

    #[test]
    fn test_extract_keywords_greeting() {
        let mut f = BurstFeedback::new("m".into(), "f".into(), "hello there".into(), "hi".into());
        f.extract_keywords();
        assert!(f.prompt_keywords.contains(&"greeting".to_string()));
    }

    #[test]
    fn test_extract_keywords_code() {
        let mut f = BurstFeedback::new(
            "m".into(),
            "f".into(),
            "write a function".into(),
            "fn()".into(),
        );
        f.extract_keywords();
        assert!(f.prompt_keywords.contains(&"code_request".to_string()));
    }

    #[test]
    fn test_extract_keywords_empty() {
        let mut f = BurstFeedback::new("m".into(), "f".into(), "ok".into(), "ok".into());
        f.extract_keywords();
        assert!(f.prompt_keywords.is_empty());
    }

    #[test]
    fn test_model_pattern_from_feedback() {
        let mut f = BurstFeedback::new("m".into(), "glm".into(), "2+2".into(), "r".into());
        f.extract_keywords();
        let f = f.with_code_fence_issue();
        let p = ModelPattern::from_feedback(&f);
        assert_eq!(p.model_family, "glm");
        assert_eq!(p.issue_type, FeedbackIssue::UnwantedCodeFence);
        assert_eq!(p.observation_count, 1);
        assert_eq!(p.success_rate, 0.0);
    }

    #[test]
    fn test_model_pattern_record_outcome() {
        let f = BurstFeedback::new("m".into(), "glm".into(), "p".into(), "r".into());
        let mut p = ModelPattern::from_feedback(&f);
        assert_eq!(p.success_rate, 0.0);
        p.record_outcome(true);
        assert!((p.success_rate - 0.2).abs() < f64::EPSILON);
        p.record_outcome(true);
        assert!(p.success_rate > 0.2);
        p.record_outcome(false);
        assert!(p.success_rate < 0.36);
    }
}
