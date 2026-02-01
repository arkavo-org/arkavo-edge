//! Tool pattern cache for storing learned tool call patterns
//!
//! Stores tool_format lessons received from local learning or gossip for prompt injection.
//! Uses generic Lesson objects filtered by category.

use super::tool_pattern_observer::TOOL_FORMAT_CATEGORY;
use arkavo_router::learning::{Lesson, ToolCallFormat};
use std::collections::HashMap;
use tracing::{debug, info};

/// Cache of learned tool call patterns for few-shot prompt injection
pub struct ToolPatternCache {
    /// Best lesson per tool (highest confidence)
    lessons_by_tool: HashMap<String, Lesson>,
}

impl Default for ToolPatternCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolPatternCache {
    /// Create a new empty cache
    pub fn new() -> Self {
        Self {
            lessons_by_tool: HashMap::new(),
        }
    }

    /// Add or update a tool pattern lesson
    /// Only accepts lessons with category == "tool_format"
    pub fn add_lesson(&mut self, lesson: Lesson) {
        if lesson.category != TOOL_FORMAT_CATEGORY {
            debug!(
                category = %lesson.category,
                "Ignoring non-tool_format lesson"
            );
            return;
        }

        // Extract tool_name from metadata
        let tool_name = match lesson.pattern.metadata.as_ref() {
            Some(meta) => match meta.get("tool_name") {
                Some(serde_json::Value::String(name)) => name.clone(),
                _ => {
                    debug!("Tool format lesson missing tool_name in metadata");
                    return;
                }
            },
            None => {
                debug!("Tool format lesson has no metadata");
                return;
            }
        };

        // Only add if better than existing or new
        if let Some(existing) = self.lessons_by_tool.get(&tool_name) {
            if lesson.confidence > existing.confidence
                || lesson.episode_count > existing.episode_count
            {
                info!(
                    tool = %tool_name,
                    confidence = lesson.confidence,
                    evidence = lesson.episode_count,
                    "Updating tool pattern (better than existing)"
                );
                self.lessons_by_tool.insert(tool_name, lesson);
            } else {
                debug!(
                    tool = %tool_name,
                    "Skipping tool pattern (not better than existing)"
                );
            }
        } else {
            info!(
                tool = %tool_name,
                confidence = lesson.confidence,
                evidence = lesson.episode_count,
                "Added new tool pattern to cache"
            );
            self.lessons_by_tool.insert(tool_name, lesson);
        }
    }

    /// Get the lesson for a tool
    pub fn get_lesson(&self, tool_name: &str) -> Option<&Lesson> {
        self.lessons_by_tool.get(tool_name)
    }

    /// Get lessons for multiple tools
    pub fn get_lessons_for_tools(&self, tool_names: &[String]) -> Vec<&Lesson> {
        tool_names
            .iter()
            .filter_map(|name| self.lessons_by_tool.get(name))
            .collect()
    }

    /// Generate few-shot examples for prompt injection
    pub fn get_few_shot_examples(
        &self,
        tool_names: &[String],
        _preferred_format: ToolCallFormat,
    ) -> String {
        let lessons = self.get_lessons_for_tools(tool_names);
        if lessons.is_empty() {
            return String::new();
        }

        let mut examples = Vec::new();
        for lesson in lessons {
            // Get example_invocation from metadata
            if let Some(meta) = &lesson.pattern.metadata
                && let Some(serde_json::Value::String(example)) = meta.get("example_invocation")
            {
                examples.push(example.clone());
            }
        }

        if examples.is_empty() {
            String::new()
        } else {
            format!(
                "Here are examples of successful tool calls:\n\n{}\n",
                examples.join("\n\n")
            )
        }
    }

    /// Get the number of cached lessons
    pub fn pattern_count(&self) -> usize {
        self.lessons_by_tool.len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.lessons_by_tool.is_empty()
    }

    /// Clear all cached patterns
    pub fn clear(&mut self) {
        self.lessons_by_tool.clear();
    }

    /// Get all tool names with cached lessons
    pub fn tool_names(&self) -> Vec<String> {
        self.lessons_by_tool.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_router::learning::LessonPattern;

    fn make_test_lesson(tool_name: &str, confidence: f64, evidence: u32) -> Lesson {
        let metadata = serde_json::json!({
            "format": "fence",
            "tool_name": tool_name,
            "parameter_names": ["id"],
            "example_args": {"id": 1},
            "example_invocation": format!("Example of calling {}:\n```{}\n{{\"id\": 1}}\n```", tool_name, tool_name),
            "model_name": "test-model",
            "success_count": evidence
        });

        let pattern = LessonPattern::new(
            format!("tool_call:{}", tool_name),
            "use_fence_format".to_string(),
            "successful_tool_invocation".to_string(),
        )
        .with_metadata(metadata);

        Lesson::new(
            "agent-1".to_string(),
            "swarm-1".to_string(),
            TOOL_FORMAT_CATEGORY.to_string(),
            pattern,
            confidence,
            evidence,
        )
    }

    #[test]
    fn test_add_and_get_lesson() {
        let mut cache = ToolPatternCache::new();
        let lesson = make_test_lesson("get_sector", 0.8, 5);

        cache.add_lesson(lesson);

        assert_eq!(cache.pattern_count(), 1);
        let lesson = cache.get_lesson("get_sector").unwrap();
        assert_eq!(lesson.confidence, 0.8);
    }

    #[test]
    fn test_better_lesson_replaces_existing() {
        let mut cache = ToolPatternCache::new();

        // Add initial lesson
        cache.add_lesson(make_test_lesson("get_sector", 0.5, 2));

        // Add better lesson
        cache.add_lesson(make_test_lesson("get_sector", 0.9, 10));

        let lesson = cache.get_lesson("get_sector").unwrap();
        assert_eq!(lesson.confidence, 0.9);
        assert_eq!(lesson.episode_count, 10);
    }

    #[test]
    fn test_worse_lesson_ignored() {
        let mut cache = ToolPatternCache::new();

        // Add good lesson
        cache.add_lesson(make_test_lesson("get_sector", 0.9, 10));

        // Try to add worse lesson
        cache.add_lesson(make_test_lesson("get_sector", 0.5, 2));

        let lesson = cache.get_lesson("get_sector").unwrap();
        assert_eq!(lesson.confidence, 0.9);
    }

    #[test]
    fn test_few_shot_examples() {
        let mut cache = ToolPatternCache::new();
        cache.add_lesson(make_test_lesson("get_sector", 0.8, 5));
        cache.add_lesson(make_test_lesson("set_speed", 0.7, 3));

        let tools = vec!["get_sector".to_string(), "set_speed".to_string()];
        let examples = cache.get_few_shot_examples(&tools, ToolCallFormat::Fence);

        assert!(examples.contains("get_sector"));
        assert!(examples.contains("set_speed"));
        assert!(examples.contains("successful tool calls"));
    }

    #[test]
    fn test_ignores_non_tool_format_lessons() {
        let mut cache = ToolPatternCache::new();

        // Create a lesson with wrong category
        let pattern = LessonPattern::new(
            "some_condition".to_string(),
            "some_action".to_string(),
            "some_outcome".to_string(),
        );
        let lesson = Lesson::new(
            "agent-1".to_string(),
            "swarm-1".to_string(),
            "navigation".to_string(), // wrong category
            pattern,
            0.8,
            5,
        );

        cache.add_lesson(lesson);
        assert!(cache.is_empty());
    }
}
