//! Human teaching hooks — runtime instruction, correction, and reinforcement
//!
//! The chat path is the teaching interface. Every human message gets classified:
//! - Question → normal chat flow
//! - Instruction → create Lesson with Human source, inject PolicyCache
//! - Correction → create AntiPattern linked to last DecisionTrace
//! - Reinforcement → boost episode quality, fast-track to Canonical
//!
//! Also parses AGENTS.md at startup for constraints/suggestions (Configuration source).

use super::persistence::{Lesson, LessonPattern, LessonScope, LessonSource, MemoryTier};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Classified intent of a human message for the teaching system
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TeachingIntent {
    /// Normal question or conversation — pass through to chat
    Question,
    /// Behavioral instruction ("always do X", "prefer Y over Z")
    Instruction { scope: LessonScope },
    /// Correction of a recent action ("don't do that", "that was wrong")
    Correction { trace_id: Option<Uuid> },
    /// Positive reinforcement ("good", "yes, do more of that")
    Reinforcement,
}

/// Classify a human message into a teaching intent using cheap pattern matching.
///
/// Falls back to `Question` when ambiguous. No LLM call needed.
pub fn classify_intent(text: &str, last_trace_id: Option<Uuid>) -> TeachingIntent {
    let lower = text.to_lowercase();
    let trimmed = lower.trim();

    // Correction patterns — negation + imperative, or explicit disapproval
    if is_correction(trimmed) {
        return TeachingIntent::Correction {
            trace_id: last_trace_id,
        };
    }

    // Reinforcement patterns — short affirmative or explicit praise
    if is_reinforcement(trimmed) {
        return TeachingIntent::Reinforcement;
    }

    // Instruction patterns — imperative with scope markers
    if is_instruction(trimmed) {
        let scope = detect_scope(trimmed);
        return TeachingIntent::Instruction { scope };
    }

    TeachingIntent::Question
}

/// Create a lesson from a human instruction
pub fn create_instruction_lesson(
    instruction: &str,
    scope: LessonScope,
    agent_id: &str,
    swarm_id: &str,
    category: &str,
) -> Lesson {
    let confidence = match scope {
        LessonScope::Global => 1.0,
        LessonScope::Situational => 0.95,
        LessonScope::OneShot => 0.9,
    };

    let pattern = LessonPattern::new(
        "human_instruction".to_string(),
        instruction.to_string(),
        "human".to_string(),
    );

    let mut lesson = Lesson::new(
        agent_id.to_string(),
        swarm_id.to_string(),
        category.to_string(),
        pattern,
        confidence,
        0,
    );
    lesson.tier = MemoryTier::Canonical;
    lesson.source = LessonSource::Human;
    lesson.scope = scope;
    lesson
}

fn is_correction(text: &str) -> bool {
    let starts_negative = text.starts_with("don't ")
        || text.starts_with("do not ")
        || text.starts_with("stop ")
        || text.starts_with("no, ")
        || text.starts_with("wrong")
        || text.starts_with("that's wrong")
        || text.starts_with("that was wrong")
        || text.starts_with("bad ");

    let contains_disapproval = text.contains("shouldn't have")
        || text.contains("should not have")
        || text.contains("was incorrect")
        || text.contains("was a mistake")
        || text.contains("undo that");

    starts_negative || contains_disapproval
}

fn is_reinforcement(text: &str) -> bool {
    let short_affirm = text.len() < 30
        && (text.starts_with("good")
            || text.starts_with("yes")
            || text.starts_with("correct")
            || text.starts_with("exactly")
            || text.starts_with("perfect")
            || text.starts_with("nice"));

    let explicit_praise = text.contains("do more of that")
        || text.contains("keep doing that")
        || text.contains("that was great")
        || text.contains("that's much better")
        || text.contains("that's better")
        || text.contains("much better")
        || text.contains("well done")
        || text.contains("great job")
        || text.contains("good job");

    short_affirm || explicit_praise
}

fn is_instruction(text: &str) -> bool {
    // Constraint-style instructions
    let has_constraint_marker = text.contains("must ") || text.contains("never ");

    // Imperative instructions with "always" or preference markers
    let has_directive = text.starts_with("always ")
        || text.starts_with("prefer ")
        || text.starts_with("use ")
        || text.starts_with("prioritize ")
        || text.starts_with("when ")
        || text.starts_with("make sure ")
        || text.starts_with("ensure ");

    has_constraint_marker || has_directive
}

fn detect_scope(text: &str) -> LessonScope {
    if text.contains("always") || text.contains("never") || text.contains("must") {
        LessonScope::Global
    } else if text.contains("when ") || text.contains("if ") || text.contains("for this") {
        LessonScope::Situational
    } else if text.contains("just this once")
        || text.contains("right now")
        || text.contains("this time")
    {
        LessonScope::OneShot
    } else {
        LessonScope::Global
    }
}

// --- AGENTS.md parsing (startup configuration) ---

/// A teaching signal from a human source (AGENTS.md or runtime)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeachingSignal {
    pub source: TeachingSource,
    pub instruction: String,
    pub examples: Vec<TeachingExample>,
    pub constraints: Vec<String>,
    pub overridable: bool,
}

/// Where the teaching signal came from
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TeachingSource {
    AgentsMd { path: PathBuf, section: String },
    RuntimeCorrection { task_id: Uuid },
    ExplicitFeedback { agent_id: String },
}

/// An example provided by a human teacher
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeachingExample {
    pub input: String,
    pub expected_output: String,
    pub tool_sequence: Vec<String>,
}

/// Parse AGENTS.md content into teaching signals
pub fn parse_agents_md(content: &str, path: PathBuf) -> Vec<TeachingSignal> {
    let mut signals = Vec::new();
    let mut current_section = String::new();

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('#') {
            current_section = trimmed.trim_start_matches('#').trim().to_string();
            continue;
        }

        if trimmed.is_empty() {
            continue;
        }

        let is_constraint = is_constraint_line(trimmed);

        if is_constraint || is_suggestion_line(trimmed) {
            let signal = TeachingSignal {
                source: TeachingSource::AgentsMd {
                    path: path.clone(),
                    section: current_section.clone(),
                },
                instruction: trimmed.to_string(),
                examples: vec![],
                constraints: if is_constraint {
                    vec![trimmed.to_string()]
                } else {
                    vec![]
                },
                overridable: !is_constraint,
            };
            signals.push(signal);
        }
    }

    signals
}

/// Convert teaching signals to lessons for the policy cache
pub fn signals_to_lessons(
    signals: &[TeachingSignal],
    agent_id: &str,
    swarm_id: &str,
) -> Vec<Lesson> {
    signals
        .iter()
        .map(|signal| {
            let confidence = if signal.overridable { 0.9 } else { 1.0 };
            let pattern = LessonPattern::new(
                signal.instruction.clone(),
                if signal.overridable {
                    "suggestion".to_string()
                } else {
                    "constraint".to_string()
                },
                "human-provided guidance".to_string(),
            );

            let mut lesson = Lesson::new(
                agent_id.to_string(),
                swarm_id.to_string(),
                section_to_category(&signal.source),
                pattern,
                confidence,
                0,
            );
            lesson.tier = MemoryTier::Canonical;
            lesson.source = LessonSource::Configuration;
            lesson.scope = LessonScope::Global;
            lesson
        })
        .collect()
}

fn is_constraint_line(line: &str) -> bool {
    let upper = line.to_uppercase();
    upper.contains("MUST ") || upper.contains("NEVER ") || upper.contains("ALWAYS ")
}

fn is_suggestion_line(line: &str) -> bool {
    line.starts_with("- ") || line.starts_with("* ") || line.starts_with("• ")
}

fn section_to_category(source: &TeachingSource) -> String {
    match source {
        TeachingSource::AgentsMd { section, .. } => {
            if section.is_empty() {
                "general".to_string()
            } else {
                section.to_lowercase().replace(' ', "_")
            }
        }
        TeachingSource::RuntimeCorrection { .. } => "runtime_correction".to_string(),
        TeachingSource::ExplicitFeedback { .. } => "explicit_feedback".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_AGENTS_MD: &str = r#"# Tool Usage
- Use XML format for tool calls
- MUST validate all tool arguments before execution
- NEVER call delete operations without confirmation

# Workflow
- Prefer local models for code search
* Check test results before committing
ALWAYS run clippy before pushing
"#;

    #[test]
    fn test_parse_agents_md() {
        let signals = parse_agents_md(SAMPLE_AGENTS_MD, PathBuf::from("AGENTS.md"));
        assert!(!signals.is_empty());

        let constraints: Vec<_> = signals.iter().filter(|s| !s.overridable).collect();
        let suggestions: Vec<_> = signals.iter().filter(|s| s.overridable).collect();

        assert!(
            constraints.len() >= 3,
            "should find MUST/NEVER/ALWAYS: got {}",
            constraints.len()
        );
        assert!(
            suggestions.len() >= 3,
            "should find suggestions: got {}",
            suggestions.len()
        );
    }

    #[test]
    fn test_constraint_detection() {
        assert!(is_constraint_line("MUST validate all inputs"));
        assert!(is_constraint_line("- NEVER delete without confirm"));
        assert!(is_constraint_line("ALWAYS check tests"));
        assert!(!is_constraint_line("- Use XML format"));
        assert!(!is_constraint_line("Prefer local models"));
    }

    #[test]
    fn test_suggestion_detection() {
        assert!(is_suggestion_line("- Use XML format"));
        assert!(is_suggestion_line("* Check results"));
        assert!(!is_suggestion_line("Regular text"));
    }

    #[test]
    fn test_signals_to_lessons() {
        let signals = parse_agents_md(SAMPLE_AGENTS_MD, PathBuf::from("AGENTS.md"));
        let lessons = signals_to_lessons(&signals, "agent-1", "swarm-1");

        assert_eq!(lessons.len(), signals.len());

        // Constraints should be canonical with confidence 1.0
        let constraint_lessons: Vec<_> = lessons.iter().filter(|l| l.confidence == 1.0).collect();
        assert!(
            constraint_lessons.len() >= 3,
            "constraint lessons: {}",
            constraint_lessons.len()
        );
        for l in &constraint_lessons {
            assert_eq!(l.tier, MemoryTier::Canonical);
            assert_eq!(l.pattern.action, "constraint");
            assert_eq!(l.source, LessonSource::Configuration);
        }

        // Suggestions should be canonical with confidence 0.9
        let suggestion_lessons: Vec<_> = lessons
            .iter()
            .filter(|l| (l.confidence - 0.9).abs() < 0.01)
            .collect();
        assert!(!suggestion_lessons.is_empty());
        for l in &suggestion_lessons {
            assert_eq!(l.tier, MemoryTier::Canonical);
            assert_eq!(l.pattern.action, "suggestion");
            assert_eq!(l.source, LessonSource::Configuration);
        }
    }

    #[test]
    fn test_section_categories() {
        let signals = parse_agents_md(SAMPLE_AGENTS_MD, PathBuf::from("AGENTS.md"));

        let tool_signals: Vec<_> = signals
            .iter()
            .filter(|s| {
                matches!(&s.source, TeachingSource::AgentsMd { section, .. } if section == "Tool Usage")
            })
            .collect();
        assert!(!tool_signals.is_empty());

        let lessons = signals_to_lessons(&signals, "a", "s");
        let tool_lessons: Vec<_> = lessons
            .iter()
            .filter(|l| l.category == "tool_usage")
            .collect();
        assert!(!tool_lessons.is_empty());
    }

    #[test]
    fn test_empty_content() {
        let signals = parse_agents_md("", PathBuf::from("AGENTS.md"));
        assert!(signals.is_empty());
    }

    // --- Runtime teaching intent tests ---

    #[test]
    fn test_classify_question() {
        assert_eq!(
            classify_intent("What is the current state?", None),
            TeachingIntent::Question
        );
        assert_eq!(
            classify_intent("How many colonists are alive?", None),
            TeachingIntent::Question
        );
    }

    #[test]
    fn test_classify_instruction() {
        assert_eq!(
            classify_intent("always prioritize food stockpiles", None),
            TeachingIntent::Instruction {
                scope: LessonScope::Global
            }
        );
        assert_eq!(
            classify_intent("prefer building defenses when raids are frequent", None),
            TeachingIntent::Instruction {
                scope: LessonScope::Situational
            }
        );
        assert_eq!(
            classify_intent("never abandon wounded colonists", None),
            TeachingIntent::Instruction {
                scope: LessonScope::Global
            }
        );
    }

    #[test]
    fn test_classify_correction() {
        let tid = Uuid::new_v4();
        assert_eq!(
            classify_intent("don't do that", Some(tid)),
            TeachingIntent::Correction {
                trace_id: Some(tid)
            }
        );
        assert_eq!(
            classify_intent("that was wrong", None),
            TeachingIntent::Correction { trace_id: None }
        );
        assert_eq!(
            classify_intent("stop building solar panels", Some(tid)),
            TeachingIntent::Correction {
                trace_id: Some(tid)
            }
        );
    }

    #[test]
    fn test_classify_reinforcement() {
        assert_eq!(classify_intent("good", None), TeachingIntent::Reinforcement);
        assert_eq!(
            classify_intent("yes, do more of that", None),
            TeachingIntent::Reinforcement
        );
        assert_eq!(
            classify_intent("great job on the food strategy", None),
            TeachingIntent::Reinforcement
        );
        assert_eq!(
            classify_intent(
                "Good, that's much better. Always prioritize safe food sources.",
                None,
            ),
            TeachingIntent::Reinforcement
        );
    }

    #[test]
    fn test_scope_detection() {
        assert_eq!(detect_scope("always do X"), LessonScope::Global);
        assert_eq!(detect_scope("never do Y"), LessonScope::Global);
        assert_eq!(
            detect_scope("when raiding, prefer melee"),
            LessonScope::Situational
        );
        assert_eq!(
            detect_scope("just this once, skip the check"),
            LessonScope::OneShot
        );
    }

    #[test]
    fn test_create_instruction_lesson() {
        let lesson = create_instruction_lesson(
            "always prioritize food",
            LessonScope::Global,
            "agent-1",
            "swarm-1",
            "general",
        );
        assert_eq!(lesson.source, LessonSource::Human);
        assert_eq!(lesson.scope, LessonScope::Global);
        assert_eq!(lesson.tier, MemoryTier::Canonical);
        assert_eq!(lesson.confidence, 1.0);
        assert_eq!(lesson.pattern.condition, "human_instruction");
        assert_eq!(lesson.pattern.action, "always prioritize food");
    }

    #[test]
    fn test_situational_lesson_lower_confidence() {
        let lesson = create_instruction_lesson(
            "when raiding, prefer melee",
            LessonScope::Situational,
            "agent-1",
            "swarm-1",
            "combat",
        );
        assert_eq!(lesson.confidence, 0.95);
        assert_eq!(lesson.scope, LessonScope::Situational);
    }
}
