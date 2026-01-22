//! Response feedback analyzer for adaptive prompt learning
//!
//! Analyzes model responses to detect issues and stores them as Episodes
//! in the router's LearningStore for persistent learning across sessions.

use arkavo_hrm::{BurstFeedback, FeedbackIssue};
use arkavo_router::learning::{
    Episode, EpisodeOutcome, LearningStore, Observation, QualityMetrics,
};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::OnceCell;
use uuid::Uuid;

/// Global learning store for persistent feedback storage
static LEARNING_STORE: OnceCell<Arc<LearningStore>> = OnceCell::const_new();

/// Get the database path for feedback storage
fn get_db_path() -> PathBuf {
    let mut path = PathBuf::from(".arkavo");
    path.push("learning");
    path.push("feedback.db");
    path
}

/// Initialize the global learning store
async fn init_learning_store()
-> Result<Arc<LearningStore>, Box<dyn std::error::Error + Send + Sync>> {
    let db_path = get_db_path();

    // Ensure directory exists
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let store = LearningStore::new(&db_path).await.map_err(|e| {
        Box::new(std::io::Error::other(format!("Failed to init store: {e}")))
            as Box<dyn std::error::Error + Send + Sync>
    })?;
    Ok(Arc::new(store))
}

/// Get or initialize the learning store
pub async fn get_learning_store()
-> Result<Arc<LearningStore>, Box<dyn std::error::Error + Send + Sync>> {
    LEARNING_STORE
        .get_or_try_init(init_learning_store)
        .await
        .cloned()
}

/// Analyzer for detecting issues in model responses
pub struct ResponseAnalyzer {
    model_name: String,
    model_family: String,
}

impl ResponseAnalyzer {
    /// Create a new analyzer for a specific model
    pub fn new(model_name: &str) -> Self {
        let model_family = detect_model_family(model_name);
        Self {
            model_name: model_name.to_string(),
            model_family,
        }
    }

    /// Analyze a response and create feedback if issues are detected
    pub fn analyze(&self, prompt: &str, response: &str) -> Option<BurstFeedback> {
        let mut feedback = BurstFeedback::new(
            self.model_name.clone(),
            self.model_family.clone(),
            prompt.to_string(),
            response.to_string(),
        );
        feedback.extract_keywords();

        if let Some(issue) = self.detect_issue(prompt, response) {
            match issue {
                DetectedIssue::UnwantedCodeFence => {
                    return Some(feedback.with_code_fence_issue());
                }
                DetectedIssue::OutputLoop => {
                    return Some(feedback.with_loop_issue());
                }
                DetectedIssue::WrongExpert(expected) => {
                    return Some(feedback.with_wrong_expert(&expected));
                }
                DetectedIssue::Timeout => {
                    let mut f = feedback;
                    f.issue = FeedbackIssue::EmptyOrTimeout;
                    f.issue_description = "Response timed out".to_string();
                    return Some(f);
                }
            }
        }

        None
    }

    /// Detect specific issues in the response
    fn detect_issue(&self, prompt: &str, response: &str) -> Option<DetectedIssue> {
        let lower_prompt = prompt.to_lowercase();
        let lower_response = response.to_lowercase();

        // Check for timeout/empty
        if response.is_empty() || response.trim().is_empty() {
            return Some(DetectedIssue::Timeout);
        }

        // Check for unwanted code fences on simple queries
        if self.is_simple_query(&lower_prompt) && self.has_code_fence(response) {
            return Some(DetectedIssue::UnwantedCodeFence);
        }

        // Check for output loops (repetition)
        if self.has_repetition(response) {
            return Some(DetectedIssue::OutputLoop);
        }

        // Check for wrong expert routing (GLM-specific)
        if self.model_family == "glm"
            && self.is_chat_query(&lower_prompt)
            && self.has_code_indicators(&lower_response)
        {
            return Some(DetectedIssue::WrongExpert("conversation".to_string()));
        }

        None
    }

    /// Check if the prompt is a simple query that shouldn't need code
    fn is_simple_query(&self, prompt: &str) -> bool {
        // Greetings
        if prompt.starts_with("hi") || prompt.starts_with("hello") || prompt.starts_with("hey") {
            return true;
        }

        // Simple math
        let math_patterns = ["what is", "calculate", "compute", "how much"];
        let contains_math = math_patterns.iter().any(|p| prompt.contains(p));
        let has_numbers = prompt.chars().any(|c| c.is_ascii_digit());
        if contains_math && has_numbers && prompt.len() < 50 {
            return true;
        }

        // Simple factual questions
        let factual = ["capital of", "who is", "when was", "where is"];
        if factual.iter().any(|p| prompt.contains(p)) {
            return true;
        }

        false
    }

    /// Check if response contains code fences
    fn has_code_fence(&self, response: &str) -> bool {
        response.contains("```")
    }

    /// Check for repetitive patterns indicating a loop
    fn has_repetition(&self, response: &str) -> bool {
        let lines: Vec<&str> = response.lines().collect();
        if lines.len() < 5 {
            return false;
        }

        // Check for repeated consecutive lines
        let mut repeat_count = 1;
        for i in 1..lines.len() {
            if lines[i] == lines[i - 1] && !lines[i].trim().is_empty() {
                repeat_count += 1;
                if repeat_count >= 3 {
                    return true;
                }
            } else {
                repeat_count = 1;
            }
        }

        // Check for Q&A loop pattern (GLM often generates "What is X+Y?" sequences)
        let question_count = lines
            .iter()
            .filter(|l| l.contains("What is") || l.contains("what is"))
            .count();
        if question_count >= 3 {
            return true;
        }

        // Check for response significantly longer than expected
        if lines.len() > 10 {
            let short_lines = lines.iter().filter(|l| l.len() < 20).count();
            if short_lines > lines.len() / 2 {
                return true;
            }
        }

        // Check for repeated phrases
        let words: Vec<&str> = response.split_whitespace().collect();
        if words.len() > 20 {
            let chunk_size = 5;
            for i in 0..(words.len() - chunk_size * 2) {
                let chunk1 = &words[i..i + chunk_size];
                let chunk2 = &words[i + chunk_size..i + chunk_size * 2];
                if chunk1 == chunk2 {
                    return true;
                }
            }
        }

        false
    }

    /// Check if this is a conversational query
    fn is_chat_query(&self, prompt: &str) -> bool {
        let chat_indicators = [
            "hi",
            "hello",
            "hey",
            "thanks",
            "thank you",
            "how are",
            "what's up",
            "tell me about",
            "explain",
        ];
        chat_indicators.iter().any(|p| prompt.contains(p))
    }

    /// Check if response has coding/tool expert indicators
    fn has_code_indicators(&self, response: &str) -> bool {
        response.contains("def ")
            || response.contains("import ")
            || response.contains("print(")
            || response.contains("```python")
            || response.contains("```")
    }

    /// Convert feedback issue to Episode category
    fn issue_to_category(issue: FeedbackIssue, model_family: &str) -> String {
        let issue_name = match issue {
            FeedbackIssue::UnwantedCodeFence => "code_fence",
            FeedbackIssue::HallucinatedTool => "hallucinated_tool",
            FeedbackIssue::InvalidToolFormat => "invalid_format",
            FeedbackIssue::UnexpectedRefusal => "refusal",
            FeedbackIssue::OutputLoop => "loop",
            FeedbackIssue::WrongExpertRouting => "wrong_expert",
            FeedbackIssue::EmptyOrTimeout => "timeout",
            FeedbackIssue::Correct => "correct",
        };
        format!("model:{model_family}:{issue_name}")
    }
}

/// Types of issues that can be detected
#[derive(Debug, Clone)]
enum DetectedIssue {
    UnwantedCodeFence,
    OutputLoop,
    WrongExpert(String),
    Timeout,
}

/// Extract just the first meaningful answer from a potentially loopy response
/// For math queries, this returns just the number on the first line
pub fn extract_first_answer(response: &str, is_math_query: bool) -> String {
    let trimmed = response.trim();

    if trimmed.is_empty() {
        return String::new();
    }

    // For math queries, try to extract just the number
    if is_math_query {
        // Get first line
        let first_line = trimmed.lines().next().unwrap_or(trimmed);
        // Try to parse as number or return the first line
        let clean = first_line.trim();
        // If it looks like a number (possibly negative, with decimals), return it
        if clean
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c == ' ')
        {
            return clean.to_string();
        }
        // Otherwise return first line
        return first_line.to_string();
    }

    // For non-math, return everything before repetition starts
    let lines: Vec<&str> = trimmed.lines().collect();
    if lines.len() <= 3 {
        return trimmed.to_string();
    }

    // Find where repetition starts
    let mut result_lines = Vec::new();
    let mut seen_patterns = std::collections::HashSet::new();

    for line in lines {
        let normalized = line.trim().to_lowercase();
        if normalized.is_empty() {
            result_lines.push(line);
            continue;
        }
        if seen_patterns.contains(&normalized) {
            // Repetition detected, stop here
            break;
        }
        seen_patterns.insert(normalized);
        result_lines.push(line);
    }

    result_lines.join("\n")
}

/// Check if a prompt is a simple math query
pub fn is_simple_math_query(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    lower.contains("what is")
        && (lower.contains('+')
            || lower.contains('-')
            || lower.contains('*')
            || lower.contains('/'))
        && prompt.len() < 50
}

/// Detect model family from name
fn detect_model_family(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.contains("glm") {
        "glm".to_string()
    } else if lower.contains("gemma") {
        "gemma".to_string()
    } else if lower.contains("qwen") {
        "qwen".to_string()
    } else if lower.contains("mistral") || lower.contains("ministral") {
        "mistral".to_string()
    } else if lower.contains("deepseek") {
        "deepseek".to_string()
    } else {
        "unknown".to_string()
    }
}

/// Record model feedback as an Episode in the learning store
pub async fn record_model_feedback(
    model_name: &str,
    prompt: &str,
    response: &str,
) -> Result<Option<FeedbackIssue>, Box<dyn std::error::Error + Send + Sync>> {
    let analyzer = ResponseAnalyzer::new(model_name);

    if let Some(feedback) = analyzer.analyze(prompt, response) {
        let issue = feedback.issue;
        let category = ResponseAnalyzer::issue_to_category(feedback.issue, &analyzer.model_family);

        // Create Episode from feedback
        let observation = Observation::new(
            serde_json::json!({
                "prompt": prompt,
                "model": model_name,
                "keywords": feedback.prompt_keywords,
            }),
            format!("generate_response:{}", analyzer.model_family),
            serde_json::json!({
                "response_length": response.len(),
                "issue": format!("{:?}", feedback.issue),
            }),
            vec![],
        );

        let outcome = EpisodeOutcome::new(
            false, // It's an issue, so not successful
            QualityMetrics::new(0.0, 0.0, 0.0),
            0,
            0.0,
        );

        let episode = Episode::new(
            analyzer.model_name.clone(),
            "local".to_string(), // Local swarm
            Uuid::new_v4(),      // Unique task ID
            category,
            observation,
            outcome,
        );

        // Store the episode
        if let Ok(store) = get_learning_store().await {
            if let Err(e) = store.store_episode(&episode).await {
                tracing::warn!("Failed to store feedback episode: {e}");
            } else {
                tracing::debug!(
                    issue = ?issue,
                    model = %analyzer.model_family,
                    "Stored feedback episode"
                );
            }
        }

        return Ok(Some(issue));
    }

    Ok(None)
}

/// Record a timeout event
pub async fn record_timeout_feedback(
    model_name: &str,
    prompt: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let model_family = detect_model_family(model_name);
    let category = format!("model:{model_family}:timeout");

    let observation = Observation::new(
        serde_json::json!({
            "prompt": prompt,
            "model": model_name,
        }),
        format!("generate_response:{model_family}"),
        serde_json::json!({
            "timeout": true,
        }),
        vec![],
    );

    let outcome = EpisodeOutcome::failure();

    let episode = Episode::new(
        model_name.to_string(),
        "local".to_string(),
        Uuid::new_v4(),
        category,
        observation,
        outcome,
    );

    if let Ok(store) = get_learning_store().await {
        store.store_episode(&episode).await.map_err(|e| {
            Box::new(std::io::Error::other(format!("Store error: {e}")))
                as Box<dyn std::error::Error + Send + Sync>
        })?;
    }

    Ok(())
}

/// Get count of issues by category for a model
pub async fn get_model_issue_counts(
    model_family: &str,
) -> Result<Vec<(String, usize)>, Box<dyn std::error::Error + Send + Sync>> {
    let store = get_learning_store().await?;
    let mut counts = Vec::new();

    // Check common issue categories
    let categories = [
        "loop",
        "code_fence",
        "timeout",
        "wrong_expert",
        "hallucinated_tool",
    ];

    for issue in categories {
        let category = format!("model:{model_family}:{issue}");
        let episodes = store
            .get_episodes_by_category(&category, 1000)
            .await
            .unwrap_or_default();
        if !episodes.is_empty() {
            counts.push((issue.to_string(), episodes.len()));
        }
    }

    Ok(counts)
}

/// Pattern adjustment result with optional parameters
#[derive(Debug, Clone)]
pub struct PatternAdjustment {
    /// Prompt prefix to add
    pub prompt_prefix: String,
    /// Suggested max tokens (for loopy models)
    pub max_tokens: Option<u32>,
}

/// Check for pattern-based prompt adjustment
pub async fn check_for_pattern_adjustment(
    model_name: &str,
    prompt: &str,
) -> Result<Option<PatternAdjustment>, Box<dyn std::error::Error + Send + Sync>> {
    let model_family = detect_model_family(model_name);
    let lower = prompt.to_lowercase();

    // Check if this prompt matches known problematic patterns
    let is_math = lower.contains("what is")
        && (lower.contains('+')
            || lower.contains('-')
            || lower.contains('*')
            || lower.contains('/'));

    tracing::debug!(
        model = %model_name,
        family = %model_family,
        prompt = %prompt,
        is_math = %is_math,
        "Checking pattern adjustment"
    );

    if is_math {
        // Check if we have loop issues for this model
        let category = format!("model:{model_family}:loop");
        if let Ok(store) = get_learning_store().await {
            let episodes = store
                .get_episodes_by_category(&category, 10)
                .await
                .unwrap_or_default();

            tracing::debug!(
                category = %category,
                episode_count = episodes.len(),
                "Checked for loop episodes"
            );

            if episodes.len() >= 2 {
                // We've seen loops with math questions, apply aggressive mitigation
                return Ok(Some(PatternAdjustment {
                    prompt_prefix: String::new(), // Don't add prefix, it gets echoed
                    max_tokens: Some(20),         // Very short - just enough for the number
                }));
            }
        }
    }

    Ok(None)
}

/// Synchronous wrapper for pattern adjustment check
///
/// Uses block_in_place + block_on which is the recommended pattern for
/// calling async code from sync context within a tokio multi-threaded runtime.
#[allow(clippy::disallowed_methods)]
pub fn check_pattern_adjustment_sync(model_name: &str, prompt: &str) -> Option<PatternAdjustment> {
    tracing::debug!(model = %model_name, prompt = %prompt, "check_pattern_adjustment_sync called");
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| {
            handle.block_on(async {
                check_for_pattern_adjustment(model_name, prompt)
                    .await
                    .ok()
                    .flatten()
            })
        })
    } else {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?;
        rt.block_on(async {
            check_for_pattern_adjustment(model_name, prompt)
                .await
                .ok()
                .flatten()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_code_fence() {
        let analyzer = ResponseAnalyzer::new("glm-4.7-flash");
        assert!(analyzer.has_code_fence("```python\nprint('hello')\n```"));
        assert!(!analyzer.has_code_fence("Hello, world!"));
    }

    #[test]
    fn test_detect_simple_query() {
        let analyzer = ResponseAnalyzer::new("glm-4.7-flash");
        assert!(analyzer.is_simple_query("hello"));
        assert!(analyzer.is_simple_query("what is 2+2"));
        assert!(analyzer.is_simple_query("capital of france"));
        assert!(!analyzer.is_simple_query("write a function to sort an array"));
    }

    #[test]
    fn test_detect_repetition() {
        let analyzer = ResponseAnalyzer::new("glm-4.7-flash");
        let repeated = "line\nline\nline\nline\nline";
        assert!(analyzer.has_repetition(repeated));

        let normal = "This is a normal response\nWith multiple lines\nNo repetition";
        assert!(!analyzer.has_repetition(normal));
    }

    #[test]
    fn test_analyze_code_fence_issue() {
        let analyzer = ResponseAnalyzer::new("glm-4.7-flash");
        let feedback = analyzer.analyze("what is 2+2", "```python\nprint(2+2)\n```");
        assert!(feedback.is_some());
        let f = feedback.unwrap();
        assert_eq!(f.issue, FeedbackIssue::UnwantedCodeFence);
    }

    #[test]
    fn test_analyze_correct_response() {
        let analyzer = ResponseAnalyzer::new("glm-4.7-flash");
        let feedback = analyzer.analyze("what is 2+2", "4");
        assert!(feedback.is_none());
    }

    #[test]
    fn test_issue_to_category() {
        assert_eq!(
            ResponseAnalyzer::issue_to_category(FeedbackIssue::OutputLoop, "glm"),
            "model:glm:loop"
        );
        assert_eq!(
            ResponseAnalyzer::issue_to_category(FeedbackIssue::UnwantedCodeFence, "gemma"),
            "model:gemma:code_fence"
        );
    }
}
