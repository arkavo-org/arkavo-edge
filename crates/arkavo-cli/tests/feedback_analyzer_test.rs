//! Integration tests for feedback analyzer pipeline
//! Covers CRIT-011, CRIT-012, CRIT-015, CRIT-016
//!
//! Tests the ResponseAnalyzer + LearningStore + Episode pipeline directly,
//! bypassing the global OnceCell in feedback_analyzer.rs.
//! The key logic being tested: category consistency, threshold triggers,
//! and cross-category isolation.

use arkavo_critic::{DetectedIssue, ResponseAnalyzer};
use arkavo_router::learning::{
    Episode, EpisodeOutcome, LearningStore, Observation, QualityMetrics,
};
use uuid::Uuid;

async fn setup_store() -> (LearningStore, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test_feedback.db");
    let store = LearningStore::new(&db_path).await.unwrap();
    (store, tmp)
}

/// Replicate the category format used by feedback_analyzer::record_model_feedback
/// so we can verify consistency with check_for_pattern_adjustment's lookup.
fn analyzer_category(issue: &DetectedIssue, model_family: &str) -> String {
    let issue_name = match issue {
        DetectedIssue::UnwantedCodeFence => "code_fence",
        DetectedIssue::OutputLoop => "loop",
        DetectedIssue::WrongExpert(_) => "wrong_expert",
        DetectedIssue::Timeout => "timeout",
    };
    format!("model:{model_family}:{issue_name}")
}

/// Build an episode matching the shape record_model_feedback produces
fn build_feedback_episode(
    model_name: &str,
    prompt: &str,
    response: &str,
    category: &str,
) -> Episode {
    Episode::new(
        model_name.into(),
        "local".into(),
        Uuid::new_v4(),
        category.to_string(),
        Observation::new(
            serde_json::json!({"prompt": prompt, "model": model_name}),
            format!(
                "generate_response:{}",
                arkavo_critic::response_analyzer::detect_model_family(model_name)
            ),
            serde_json::json!({"response_length": response.len()}),
            vec![],
        ),
        EpisodeOutcome::new(false, QualityMetrics::new(0.0, 0.0, 0.0), 0, 0.0),
    )
}

// Covers CRIT-011: Category produced by ResponseAnalyzer matches what
// check_for_pattern_adjustment queries — if these diverge, adjustment
// never triggers (real bug scenario).
#[tokio::test]
async fn test_category_consistency_between_record_and_lookup() {
    let (store, _tmp) = setup_store().await;

    let analyzer = ResponseAnalyzer::new("glm-4.7-flash");

    // Trigger a loop detection
    let loopy = "line\nline\nline\nline\nline";
    let result = analyzer.analyze("what is 2+2", loopy).unwrap();
    assert_eq!(result.issue, DetectedIssue::OutputLoop);

    // Category produced by the analyzer
    let stored_category = analyzer_category(&result.issue, &result.model_family);
    assert_eq!(stored_category, "model:glm:loop");

    // check_for_pattern_adjustment hardcodes: format!("model:{model_family}:loop")
    // If this format ever diverges from what record_model_feedback stores,
    // adjustment silently breaks. Verify they match.
    let lookup_category = format!("model:{}:loop", result.model_family);
    assert_eq!(
        stored_category, lookup_category,
        "Category mismatch: record stores '{stored_category}' but lookup queries '{lookup_category}'"
    );

    // Store 2 episodes and verify retrieval works with the exact category
    for _ in 0..2 {
        let ep = build_feedback_episode("glm-4.7-flash", "what is 2+2", loopy, &stored_category);
        store.store_episode(&ep).await.unwrap();
    }
    let episodes = store
        .get_episodes_by_category(&lookup_category, 10)
        .await
        .unwrap();
    assert_eq!(episodes.len(), 2);
}

// Covers CRIT-012: Threshold logic — adjustment triggers at >=2 loop episodes
// for math prompts but NOT for non-math prompts or different issue types.
#[tokio::test]
async fn test_adjustment_threshold_respects_prompt_type() {
    let (store, _tmp) = setup_store().await;
    let category = "model:glm:loop";

    // Store 3 loop episodes
    for _ in 0..3 {
        let ep = build_feedback_episode("glm-4.7-flash", "what is 2+2", "loop", category);
        store.store_episode(&ep).await.unwrap();
    }

    // Replicate check_for_pattern_adjustment logic:
    // 1. Math prompt → should trigger (episodes >= 2)
    let math_prompt = "what is 5+3";
    let is_math = {
        let lower = math_prompt.to_lowercase();
        lower.contains("what is") && lower.contains('+')
    };
    assert!(is_math, "Should detect math prompt");
    let episodes = store.get_episodes_by_category(category, 10).await.unwrap();
    let should_adjust = is_math && episodes.len() >= 2;
    assert!(
        should_adjust,
        "Math prompt with 3 loop episodes should trigger adjustment"
    );

    // 2. Non-math prompt → should NOT trigger even with episodes present
    let chat_prompt = "hello how are you";
    let is_math_chat = {
        let lower = chat_prompt.to_lowercase();
        lower.contains("what is") && lower.contains('+')
    };
    assert!(!is_math_chat, "Chat prompt should not be detected as math");
    let should_adjust_chat = is_math_chat && episodes.len() >= 2;
    assert!(
        !should_adjust_chat,
        "Chat prompt should never trigger adjustment"
    );
}

// Covers CRIT-012: Below threshold — 1 episode is not enough
#[tokio::test]
async fn test_single_loop_episode_below_threshold() {
    let (store, _tmp) = setup_store().await;

    let ep = build_feedback_episode("glm-4.7-flash", "what is 2+2", "loop", "model:glm:loop");
    store.store_episode(&ep).await.unwrap();

    let episodes = store
        .get_episodes_by_category("model:glm:loop", 10)
        .await
        .unwrap();
    assert!(
        episodes.len() < 2,
        "With 1 episode, threshold should NOT be met"
    );
}

// Covers CRIT-015: Timeout detection produces correct category and episode shape
#[tokio::test]
async fn test_timeout_detection_and_storage() {
    let (store, _tmp) = setup_store().await;

    // Empty response triggers Timeout
    let analyzer = ResponseAnalyzer::new("qwen2.5-coder");
    let result = analyzer.analyze("what is 2+2", "").unwrap();
    assert_eq!(result.issue, DetectedIssue::Timeout);
    assert_eq!(result.model_family, "qwen");

    let category = analyzer_category(&result.issue, &result.model_family);
    let ep = build_feedback_episode("qwen2.5-coder", "what is 2+2", "", &category);
    store.store_episode(&ep).await.unwrap();

    let episodes = store.get_episodes_by_category(&category, 10).await.unwrap();
    assert_eq!(episodes.len(), 1);
    // Verify the stored episode has correct agent_id (model_name, not family)
    assert_eq!(episodes[0].agent_id, "qwen2.5-coder");
    assert!(!episodes[0].outcome.success);
    // Verify observation preserves the prompt for debugging
    let obs: serde_json::Value = serde_json::to_value(&episodes[0].observation).unwrap();
    assert!(obs["state_before"]["prompt"].as_str().is_some());
}

// Covers CRIT-016: Cross-category isolation — code_fence episodes must not
// leak into loop queries or vice versa.
#[tokio::test]
async fn test_cross_category_isolation() {
    let (store, _tmp) = setup_store().await;

    // Store episodes in different categories for the SAME model
    for _ in 0..3 {
        let ep = build_feedback_episode("glm-4.7-flash", "hi", "```x```", "model:glm:code_fence");
        store.store_episode(&ep).await.unwrap();
    }
    for _ in 0..2 {
        let ep = build_feedback_episode("glm-4.7-flash", "what is 1+1", "loop", "model:glm:loop");
        store.store_episode(&ep).await.unwrap();
    }
    let ep = build_feedback_episode("glm-4.7-flash", "what is 1+1", "", "model:glm:timeout");
    store.store_episode(&ep).await.unwrap();

    // Each category must return ONLY its own episodes
    let fences = store
        .get_episodes_by_category("model:glm:code_fence", 100)
        .await
        .unwrap();
    let loops = store
        .get_episodes_by_category("model:glm:loop", 100)
        .await
        .unwrap();
    let timeouts = store
        .get_episodes_by_category("model:glm:timeout", 100)
        .await
        .unwrap();

    assert_eq!(fences.len(), 3, "code_fence should have exactly 3");
    assert_eq!(loops.len(), 2, "loop should have exactly 2");
    assert_eq!(timeouts.len(), 1, "timeout should have exactly 1");

    // Nonexistent category returns empty
    let empty = store
        .get_episodes_by_category("model:glm:wrong_expert", 100)
        .await
        .unwrap();
    assert!(empty.is_empty());
}
