//! LLM-based synthesis for episodes and lessons
//!
//! Converts observations into episodes and episodes into lessons using LLM.

use arkavo_llm::Message;
use arkavo_router::learning::{
    Episode, EpisodeOutcome, Lesson, LessonPattern, Observation, QualityMetrics,
};
use arkavo_router::Router;
use uuid::Uuid;

use super::episode_buffer::ToolObservation;

/// Synthesize an episode from observations using LLM
pub(super) async fn synthesize_episode(
    router: &Router,
    agent_id: &str,
    swarm_id: &str,
    observations: &[ToolObservation],
    category: &str,
) -> Result<Episode, String> {
    // Build prompt for LLM
    let obs_json = observations
        .iter()
        .map(|o| {
            serde_json::json!({
                "tool": o.tool_name,
                "args": o.args,
                "result": o.result,
                "success": o.success,
                "latency_ms": o.latency_ms
            })
        })
        .collect::<Vec<_>>();

    let has_failure = observations.iter().any(|o| !o.success);
    let total_latency: u64 = observations.iter().map(|o| o.latency_ms).sum();

    let prompt = format!(
        r#"Analyze these tool call observations and summarize as a structured episode.

Category: {}
Observations:
{}

The sequence {} a failure.

Respond with a JSON object:
{{
  "summary": "brief description of what happened",
  "key_insight": "most important learning from this sequence",
  "quality_score": 0.0-1.0 based on success and efficiency
}}"#,
        category,
        serde_json::to_string_pretty(&obs_json).unwrap_or_default(),
        if has_failure {
            "contains"
        } else {
            "does not contain"
        }
    );

    let messages = vec![Message::user(prompt)];
    let stream = router
        .route(
            "Analyze observations and create episode summary",
            messages,
            None,
        )
        .await
        .map_err(|e| format!("Router error: {e}"))?;

    let response = stream
        .complete()
        .await
        .map_err(|e| format!("Stream error: {e}"))?;

    // Parse LLM response - extract quality score if present
    let quality_score = extract_quality_score(&response.content);

    // Create episode
    let observation = Observation::new(
        serde_json::json!({"observations": obs_json}),
        format!("{} tool calls in {}", observations.len(), category),
        serde_json::json!({"summary": response.content}),
        observations.iter().map(|o| o.tool_name.clone()).collect(),
    );

    let outcome = EpisodeOutcome::new(
        !has_failure,
        QualityMetrics::new(quality_score, quality_score, 1.0),
        total_latency,
        response.cost_usd,
    );

    Ok(Episode::new(
        agent_id.to_string(),
        swarm_id.to_string(),
        Uuid::new_v4(),
        category.to_string(),
        observation,
        outcome,
    ))
}

/// Synthesize a lesson from episodes using LLM
pub(super) async fn synthesize_lesson(
    router: &Router,
    agent_id: &str,
    swarm_id: &str,
    episodes: &[Episode],
    category: &str,
    min_confidence: f64,
) -> Result<Option<Lesson>, String> {
    // Build prompt for LLM
    let eps_json = episodes
        .iter()
        .map(|e| {
            serde_json::json!({
                "category": e.task_category,
                "action": e.observation.action_taken,
                "success": e.outcome.success,
                "quality": e.outcome.quality_metrics.correctness,
                "tools": e.observation.tools_used
            })
        })
        .collect::<Vec<_>>();

    let failure_count = episodes.iter().filter(|e| !e.outcome.success).count();
    let success_count = episodes.iter().filter(|e| e.outcome.success).count();

    let prompt = format!(
        r#"Analyze these episodes and extract a reusable lesson pattern.

Category: {category}
Episodes: {} total ({success_count} successes, {failure_count} failures)
{}

Look for patterns:
- What conditions led to failures?
- What actions prevented failures?
- What invariants should be maintained?

If there's a clear pattern, respond with JSON:
{{
  "condition": "when this applies (e.g., sector_4)",
  "action": "recommended action (e.g., slow, avoid)",
  "expected_outcome": "what should happen",
  "confidence": 0.0-1.0 based on evidence strength
}}

If no clear pattern, respond with: NO_LESSON"#,
        episodes.len(),
        serde_json::to_string_pretty(&eps_json).unwrap_or_default()
    );

    let messages = vec![Message::user(prompt)];
    let stream = router
        .route("Extract lesson pattern from episodes", messages, None)
        .await
        .map_err(|e| format!("Router error: {e}"))?;

    let response = stream
        .complete()
        .await
        .map_err(|e| format!("Stream error: {e}"))?;

    // Check for no lesson
    if response.content.contains("NO_LESSON") {
        tracing::debug!("LLM found no pattern in {} episodes", episodes.len());
        return Ok(None);
    }

    // Parse the lesson pattern from response
    let pattern = parse_lesson_pattern(&response.content)?;

    // Validate confidence
    if pattern.2 < min_confidence {
        tracing::debug!("Lesson confidence too low: {}", pattern.2);
        return Ok(None);
    }

    let lesson = Lesson::new(
        agent_id.to_string(),
        swarm_id.to_string(),
        category.to_string(),
        LessonPattern::new(pattern.0, pattern.1, pattern.3),
        pattern.2,
        episodes.len() as u32,
    );

    Ok(Some(lesson))
}

/// Extract quality score from LLM response
fn extract_quality_score(content: &str) -> f64 {
    // Try to find quality_score in JSON
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(content)
        && let Some(score) = json.get("quality_score").and_then(|v| v.as_f64())
    {
        return score.clamp(0.0, 1.0);
    }
    // Default to 0.7 if we can't parse
    0.7
}

/// Parse lesson pattern from LLM response
fn parse_lesson_pattern(content: &str) -> Result<(String, String, f64, String), String> {
    // Try to extract JSON from content (may have markdown wrapping)
    let json_str = if let (Some(start), Some(end)) = (content.find('{'), content.rfind('}')) {
        &content[start..=end]
    } else {
        content
    };

    let json: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| format!("Failed to parse JSON: {e}"))?;

    let condition = json
        .get("condition")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let action = json
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("slow")
        .to_string();
    let confidence = json
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.5);
    let expected_outcome = json
        .get("expected_outcome")
        .and_then(|v| v.as_str())
        .unwrap_or("improved_outcome")
        .to_string();

    Ok((condition, action, confidence, expected_outcome))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_quality_score() {
        assert_eq!(
            extract_quality_score(r#"{"quality_score": 0.8}"#),
            0.8
        );
        assert_eq!(
            extract_quality_score(r#"{"quality_score": 1.5}"#),
            1.0
        );
        assert_eq!(extract_quality_score("invalid"), 0.7);
    }

    #[test]
    fn test_parse_lesson_pattern() {
        let content = r#"{"condition": "sector_4", "action": "slow", "confidence": 0.9, "expected_outcome": "safe"}"#;
        let result = parse_lesson_pattern(content).unwrap();
        assert_eq!(result.0, "sector_4");
        assert_eq!(result.1, "slow");
        assert_eq!(result.2, 0.9);
        assert_eq!(result.3, "safe");
    }

    #[test]
    fn test_parse_lesson_pattern_with_markdown() {
        let content = r#"```json
{"condition": "sector_5", "action": "avoid"}
```"#;
        let result = parse_lesson_pattern(content).unwrap();
        assert_eq!(result.0, "sector_5");
        assert_eq!(result.1, "avoid");
    }
}
