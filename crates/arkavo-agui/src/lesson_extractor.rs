use crate::response_judge::{FailureMode, TaskJudgment};
use arkavo_router::learning::{Lesson, LessonPattern};

pub struct LessonContext {
    pub agent_id: String,
    pub task_category: String,
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
    let (condition, action, expected) = match primary {
        FailureMode::Empty => (
            format!("Agent {} returns empty responses", ctx.agent_id),
            format!(
                "Avoid routing {} tasks to {}",
                ctx.task_category, ctx.agent_id
            ),
            "Non-empty substantive response".to_string(),
        ),
        FailureMode::Generic => (
            format!(
                "Agent {} returns generic non-answers for {}",
                ctx.agent_id, ctx.task_category
            ),
            format!(
                "Reduce routing weight for {} on {} tasks",
                ctx.agent_id, ctx.task_category
            ),
            "Substantive task-specific response".to_string(),
        ),
        FailureMode::TooShort => (
            format!(
                "Agent {} produces overly brief {} responses",
                ctx.agent_id, ctx.task_category
            ),
            format!("Prefer verbose agents for {} tasks", ctx.task_category),
            "Response proportional to task complexity".to_string(),
        ),
        FailureMode::ToolError {
            error_count,
            total_count,
        } => (
            format!(
                "Agent {} has {}/{} tool call failures on {} tasks",
                ctx.agent_id, error_count, total_count, ctx.task_category
            ),
            format!(
                "Route {} tasks to agents with reliable tool access",
                ctx.task_category
            ),
            "All tool calls succeed".to_string(),
        ),
        FailureMode::HallucinatedTool { count } => (
            format!(
                "Agent {} hallucinated {} tools on {} tasks",
                ctx.agent_id, count, ctx.task_category
            ),
            format!(
                "Avoid {} for tool-heavy {} tasks",
                ctx.agent_id, ctx.task_category
            ),
            "Only call registered tools".to_string(),
        ),
        FailureMode::OutputLoop => (
            format!(
                "Agent {} enters output loops on {} tasks",
                ctx.agent_id, ctx.task_category
            ),
            format!(
                "Reduce routing weight for {} on {}",
                ctx.agent_id, ctx.task_category
            ),
            "Non-repetitive response content".to_string(),
        ),
    };

    let pattern = LessonPattern::new(condition, action, expected);
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
        }
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
