//! Response feedback analyzer for adaptive prompt learning
//!
//! Analyzes model responses to detect issues and stores them as Episodes
//! in the router's LearningStore for persistent learning across sessions.
//! Pure analysis logic lives in `arkavo_critic::response_analyzer`.

use arkavo_critic::response_analyzer::{self, DetectedIssue};
use arkavo_hrm::{BurstFeedback, FeedbackIssue};
use arkavo_router::learning::{
    Episode, EpisodeOutcome, LearningStore, Observation, QualityMetrics,
};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::OnceCell;
use uuid::Uuid;

pub use arkavo_critic::response_analyzer::{extract_first_answer, is_simple_math_query};

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

/// Convert a DetectedIssue to a FeedbackIssue
fn detected_to_feedback_issue(issue: &DetectedIssue) -> FeedbackIssue {
    match issue {
        DetectedIssue::UnwantedCodeFence => FeedbackIssue::UnwantedCodeFence,
        DetectedIssue::OutputLoop => FeedbackIssue::OutputLoop,
        DetectedIssue::WrongExpert(_) => FeedbackIssue::WrongExpertRouting,
        DetectedIssue::Timeout => FeedbackIssue::EmptyOrTimeout,
    }
}

/// Convert FeedbackIssue to Episode category
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

/// Record model feedback as an Episode in the learning store
pub async fn record_model_feedback(
    model_name: &str,
    prompt: &str,
    response: &str,
) -> Result<Option<FeedbackIssue>, Box<dyn std::error::Error + Send + Sync>> {
    let analyzer = response_analyzer::ResponseAnalyzer::new(model_name);

    if let Some(result) = analyzer.analyze(prompt, response) {
        let feedback_issue = detected_to_feedback_issue(&result.issue);

        let mut feedback = BurstFeedback::new(
            model_name.to_string(),
            result.model_family.clone(),
            prompt.to_string(),
            response.to_string(),
        );
        feedback.extract_keywords();

        let category = issue_to_category(feedback_issue, &result.model_family);

        // Apply issue-specific metadata on feedback
        match &result.issue {
            DetectedIssue::UnwantedCodeFence => {
                feedback = feedback.with_code_fence_issue();
            }
            DetectedIssue::OutputLoop => {
                feedback = feedback.with_loop_issue();
            }
            DetectedIssue::WrongExpert(expected) => {
                feedback = feedback.with_wrong_expert(expected);
            }
            DetectedIssue::Timeout => {
                feedback.issue = FeedbackIssue::EmptyOrTimeout;
                feedback.issue_description = "Response timed out".to_string();
            }
        }

        let issue = feedback.issue;

        // Create Episode from feedback
        let observation = Observation::new(
            serde_json::json!({
                "prompt": prompt,
                "model": model_name,
                "keywords": feedback.prompt_keywords,
            }),
            format!("generate_response:{}", analyzer.model_family()),
            serde_json::json!({
                "response_length": response.len(),
                "issue": format!("{:?}", feedback.issue),
            }),
            vec![],
        );

        let outcome = EpisodeOutcome::new(false, QualityMetrics::new(0.0, 0.0, 0.0), 0, 0.0);

        let episode = Episode::new(
            analyzer.model_name().to_string(),
            "local".to_string(),
            Uuid::new_v4(),
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
                    model = %analyzer.model_family(),
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
    let model_family = response_analyzer::detect_model_family(model_name);
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
    let model_family = response_analyzer::detect_model_family(model_name);
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
                return Ok(Some(PatternAdjustment {
                    prompt_prefix: String::new(),
                    max_tokens: Some(20),
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
    use arkavo_critic::response_analyzer::{
        ResponseAnalyzer, has_code_fence, has_repetition, is_simple_query,
    };

    #[test]
    fn test_detect_code_fence() {
        assert!(has_code_fence("```python\nprint('hello')\n```"));
        assert!(!has_code_fence("Hello, world!"));
    }

    #[test]
    fn test_detect_simple_query() {
        assert!(is_simple_query("hello"));
        assert!(is_simple_query("what is 2+2"));
        assert!(is_simple_query("capital of france"));
        assert!(!is_simple_query("write a function to sort an array"));
    }

    #[test]
    fn test_detect_repetition() {
        let repeated = "line\nline\nline\nline\nline";
        assert!(has_repetition(repeated));

        let normal = "This is a normal response\nWith multiple lines\nNo repetition";
        assert!(!has_repetition(normal));
    }

    #[test]
    fn test_analyze_code_fence_issue() {
        let analyzer = ResponseAnalyzer::new("glm-4.7-flash");
        let result = analyzer.analyze("what is 2+2", "```python\nprint(2+2)\n```");
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.issue, DetectedIssue::UnwantedCodeFence);
    }

    #[test]
    fn test_analyze_correct_response() {
        let analyzer = ResponseAnalyzer::new("glm-4.7-flash");
        let result = analyzer.analyze("what is 2+2", "4");
        assert!(result.is_none());
    }

    #[test]
    fn test_issue_to_category() {
        assert_eq!(
            issue_to_category(FeedbackIssue::OutputLoop, "glm"),
            "model:glm:loop"
        );
        assert_eq!(
            issue_to_category(FeedbackIssue::UnwantedCodeFence, "gemma"),
            "model:gemma:code_fence"
        );
    }
}
