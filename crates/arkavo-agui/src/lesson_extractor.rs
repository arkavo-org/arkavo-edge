use crate::response_judge::{FailureMode, TaskJudgment};
use arkavo_router::learning::{Lesson, LessonPattern};

/// Remove JSON object fragments from text to keep lesson conditions readable
fn strip_json_fragments(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut depth = 0i32;
    for ch in s.chars() {
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth = (depth - 1).max(0);
        } else if depth == 0 {
            result.push(ch);
        }
    }
    // Collapse multiple spaces left by removed JSON
    let mut prev_space = false;
    let collapsed: String = result
        .chars()
        .filter(|c| {
            if *c == ' ' {
                if prev_space {
                    return false;
                }
                prev_space = true;
            } else {
                prev_space = false;
            }
            true
        })
        .collect();
    collapsed.trim().to_string()
}

pub struct LessonContext {
    pub agent_id: String,
    pub task_category: String,
    pub task_description: Option<String>,
    pub response_snippet: Option<String>,
}

/// Extract a lesson from judge output (no LLM required)
///
/// Only produces a lesson for low-quality responses with at least one
/// structured failure mode. Maps the primary failure mode to a
/// condition/action pair for gossip propagation.
pub fn extract_lesson(judgment: &TaskJudgment, ctx: &LessonContext) -> Option<Lesson> {
    if judgment.quality_score > 0.5 || judgment.failure_modes.is_empty() {
        return None;
    }

    let primary = &judgment.failure_modes[0];
    let task_hint = ctx
        .task_description
        .as_deref()
        .map(|d| {
            let cleaned = strip_json_fragments(d);
            format!(" (task: {})", &cleaned[..cleaned.len().min(80)])
        })
        .unwrap_or_default();
    let (condition, action, expected) = match primary {
        FailureMode::Empty => (
            format!(
                "Agent {} returns empty responses for {}{}",
                ctx.agent_id, ctx.task_category, task_hint
            ),
            format!(
                "Avoid routing {} tasks to {}",
                ctx.task_category, ctx.agent_id
            ),
            "Non-empty substantive response".to_string(),
        ),
        FailureMode::Generic => (
            format!(
                "Agent {} returns generic non-answers for {}{}",
                ctx.agent_id, ctx.task_category, task_hint
            ),
            format!(
                "Reduce routing weight for {} on {} tasks",
                ctx.agent_id, ctx.task_category
            ),
            "Substantive task-specific response".to_string(),
        ),
        FailureMode::TooShort => (
            format!(
                "Agent {} produces overly brief {} responses{}",
                ctx.agent_id, ctx.task_category, task_hint
            ),
            format!("Prefer verbose agents for {} tasks", ctx.task_category),
            "Response proportional to task complexity".to_string(),
        ),
        FailureMode::ToolError {
            error_count,
            total_count,
        } => (
            format!(
                "Agent {} has {}/{} tool call failures on {} tasks{}",
                ctx.agent_id, error_count, total_count, ctx.task_category, task_hint
            ),
            format!(
                "Route {} tasks to agents with reliable tool access",
                ctx.task_category
            ),
            "All tool calls succeed".to_string(),
        ),
        FailureMode::HallucinatedTool { count } => (
            format!(
                "Agent {} hallucinated {} tools on {} tasks{}",
                ctx.agent_id, count, ctx.task_category, task_hint
            ),
            format!(
                "Avoid {} for tool-heavy {} tasks",
                ctx.agent_id, ctx.task_category
            ),
            "Only call registered tools".to_string(),
        ),
        FailureMode::OutputLoop => (
            format!(
                "Agent {} enters output loops on {} tasks{}",
                ctx.agent_id, ctx.task_category, task_hint
            ),
            format!(
                "Reduce routing weight for {} on {}",
                ctx.agent_id, ctx.task_category
            ),
            "Non-repetitive response content".to_string(),
        ),
    };

    let pattern = LessonPattern::new(condition, action, expected)
        .with_metadata(serde_json::json!({"quality_score": judgment.quality_score}));
    let confidence = 1.0 - judgment.quality_score;

    Some(Lesson::new(
        ctx.agent_id.clone(),
        "local".to_string(),
        ctx.task_category.clone(),
        pattern,
        confidence,
        1,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(agent: &str, cat: &str) -> LessonContext {
        LessonContext {
            agent_id: agent.to_string(),
            task_category: cat.to_string(),
            task_description: None,
            response_snippet: None,
        }
    }

    #[test]
    fn test_strip_json_fragments() {
        assert_eq!(strip_json_fragments("hello world"), "hello world");
        assert_eq!(
            strip_json_fragments(r#"Plant rice {"Action":{"Height":6,"Plant":"Plant_Rice"}}"#),
            "Plant rice"
        );
        assert_eq!(
            strip_json_fragments(r#"do {"a":1} and {"b":2} things"#),
            "do and things"
        );
    }

    #[test]
    fn test_no_lesson_for_good_quality() {
        let judgment = TaskJudgment {
            quality_score: 0.9,
            issues: vec![],
            failure_modes: vec![],
        };
        assert!(extract_lesson(&judgment, &ctx("a", "general")).is_none());
    }

    #[test]
    fn test_no_lesson_for_no_failure_modes() {
        let judgment = TaskJudgment {
            quality_score: 0.3,
            issues: vec!["Something".into()],
            failure_modes: vec![],
        };
        assert!(extract_lesson(&judgment, &ctx("a", "general")).is_none());
    }

    #[test]
    fn test_lesson_from_empty_response() {
        let judgment = TaskJudgment {
            quality_score: 0.0,
            issues: vec!["Empty response".into()],
            failure_modes: vec![FailureMode::Empty],
        };
        let lesson = extract_lesson(&judgment, &ctx("agent-1", "security_scan")).unwrap();
        assert_eq!(lesson.category, "security_scan");
        assert!(lesson.pattern.condition.contains("empty"));
        assert!((lesson.confidence - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_lesson_from_tool_errors() {
        let judgment = TaskJudgment {
            quality_score: 0.2,
            issues: vec!["3/4 tool calls failed".into()],
            failure_modes: vec![FailureMode::ToolError {
                error_count: 3,
                total_count: 4,
            }],
        };
        let lesson = extract_lesson(&judgment, &ctx("agent-2", "code_search")).unwrap();
        assert!(lesson.pattern.condition.contains("3/4"));
        assert!((lesson.confidence - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn test_lesson_from_hallucination() {
        let judgment = TaskJudgment {
            quality_score: 0.3,
            issues: vec!["2 hallucinations".into()],
            failure_modes: vec![FailureMode::HallucinatedTool { count: 2 }],
        };
        let lesson = extract_lesson(&judgment, &ctx("agent-3", "backend_api")).unwrap();
        assert!(lesson.pattern.condition.contains("hallucinated"));
        assert!(lesson.pattern.action.contains("tool-heavy"));
    }

    #[test]
    fn test_lesson_metadata_contains_quality_score() {
        let judgment = TaskJudgment {
            quality_score: 0.3,
            issues: vec!["Generic".into()],
            failure_modes: vec![FailureMode::Generic],
        };
        let lesson = extract_lesson(&judgment, &ctx("a", "code")).unwrap();
        let meta = lesson
            .pattern
            .metadata
            .as_ref()
            .expect("metadata should be set");
        let score = meta
            .get("quality_score")
            .and_then(|v| v.as_f64())
            .expect("quality_score should be a number");
        assert!((score - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn test_lesson_confidence_inversely_proportional() {
        let low = TaskJudgment {
            quality_score: 0.1,
            issues: vec!["Generic".into()],
            failure_modes: vec![FailureMode::Generic],
        };
        let mid = TaskJudgment {
            quality_score: 0.4,
            issues: vec!["Generic".into()],
            failure_modes: vec![FailureMode::Generic],
        };
        let l1 = extract_lesson(&low, &ctx("a", "general")).unwrap();
        let l2 = extract_lesson(&mid, &ctx("a", "general")).unwrap();
        assert!(l1.confidence > l2.confidence);
    }
}
