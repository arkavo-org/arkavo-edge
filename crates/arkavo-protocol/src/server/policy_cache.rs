//! Policy cache for fast lesson lookup by sector and category
//!
//! Indexes lessons by sector ID for behavior policy checks,
//! and by category for tool format lessons.

use std::collections::HashMap;

use arkavo_router::learning::Lesson;
use uuid::Uuid;

use super::tool_pattern_observer::TOOL_FORMAT_CATEGORY;

/// Cache of lessons indexed by sector for fast policy lookup
pub struct PolicyCache {
    /// Lessons indexed by sector ID
    lessons_by_sector: HashMap<String, Vec<Lesson>>,
    /// All lessons by ID for quick lookup
    lessons_by_id: HashMap<Uuid, Lesson>,
    /// Tool format lessons indexed by tool name
    tool_format_lessons: HashMap<String, Lesson>,
}

impl Default for PolicyCache {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicyCache {
    /// Create a new empty policy cache
    pub fn new() -> Self {
        Self {
            lessons_by_sector: HashMap::new(),
            lessons_by_id: HashMap::new(),
            tool_format_lessons: HashMap::new(),
        }
    }

    /// Add a lesson to the cache
    pub fn add_lesson(&mut self, lesson: Lesson) {
        // Handle tool_format lessons specially
        if lesson.category == TOOL_FORMAT_CATEGORY {
            if let Some(tool_name) = Self::extract_tool_name(&lesson) {
                // Only keep the best lesson per tool
                if let Some(existing) = self.tool_format_lessons.get(&tool_name)
                    && lesson.confidence <= existing.confidence
                    && lesson.episode_count <= existing.episode_count
                {
                    tracing::debug!(
                        tool = %tool_name,
                        "Skipping tool_format lesson (not better than existing)"
                    );
                    return;
                }

                tracing::info!(
                    lesson_id = %lesson.id,
                    tool = %tool_name,
                    confidence = lesson.confidence,
                    "Tool format lesson added to policy cache"
                );
                self.tool_format_lessons.insert(tool_name, lesson.clone());
            }
        } else {
            // Extract sector from condition (simple parsing)
            let sector_id = Self::extract_sector(&lesson.pattern.condition);

            if let Some(ref sector) = sector_id {
                self.lessons_by_sector
                    .entry(sector.clone())
                    .or_default()
                    .push(lesson.clone());
            }

            tracing::info!(
                lesson_id = %lesson.id,
                category = %lesson.category,
                action = %lesson.pattern.action,
                condition = %lesson.pattern.condition,
                sector = ?sector_id,
                total_cached = self.lessons_by_id.len() + 1,
                "Lesson added to policy cache"
            );
        }

        self.lessons_by_id.insert(lesson.id, lesson);
    }

    /// Extract tool name from a tool_format lesson's metadata
    fn extract_tool_name(lesson: &Lesson) -> Option<String> {
        lesson
            .pattern
            .metadata
            .as_ref()
            .and_then(|m| m.get("tool_name"))
            .and_then(|v| v.as_str())
            .map(String::from)
    }

    /// Check if there's a slowdown lesson for a sector
    pub fn should_slowdown(&self, sector_id: &str) -> Option<&Lesson> {
        let result = self.lessons_by_sector.get(sector_id).and_then(|lessons| {
            lessons
                .iter()
                .find(|l| l.pattern.action.to_lowercase().contains("slow"))
        });

        if result.is_some() {
            tracing::debug!(
                sector_id = %sector_id,
                advice = "slowdown",
                "Policy check: slowdown advised"
            );
        }

        result
    }

    /// Check if there's an avoid lesson for a sector
    pub fn should_avoid(&self, sector_id: &str) -> Option<&Lesson> {
        let result = self.lessons_by_sector.get(sector_id).and_then(|lessons| {
            lessons
                .iter()
                .find(|l| l.pattern.action.to_lowercase().contains("avoid"))
        });

        if result.is_some() {
            tracing::debug!(
                sector_id = %sector_id,
                advice = "avoid",
                "Policy check: avoidance advised"
            );
        }

        result
    }

    /// Get a lesson by ID
    pub fn get_lesson(&self, lesson_id: &Uuid) -> Option<&Lesson> {
        self.lessons_by_id.get(lesson_id)
    }

    /// Get all lessons for a sector
    pub fn get_lessons_for_sector(&self, sector_id: &str) -> Vec<&Lesson> {
        self.lessons_by_sector
            .get(sector_id)
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// Get total lesson count
    pub fn len(&self) -> usize {
        self.lessons_by_id.len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.lessons_by_id.is_empty()
    }

    /// Get tool format lessons for specific tools
    pub fn get_tool_format_lessons(&self, tool_names: &[String]) -> Vec<&Lesson> {
        tool_names
            .iter()
            .filter_map(|name| self.tool_format_lessons.get(name))
            .collect()
    }

    /// Get few-shot examples for prompt injection based on learned tool patterns
    pub fn get_few_shot_examples(&self, tool_names: &[String]) -> String {
        let lessons = self.get_tool_format_lessons(tool_names);
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

    /// Get the number of cached tool format lessons
    pub fn tool_format_lesson_count(&self) -> usize {
        self.tool_format_lessons.len()
    }

    /// Extract sector ID from a condition string
    /// Supports formats: "sector_4", "sector_id == 4", "IF sector_4"
    fn extract_sector(condition: &str) -> Option<String> {
        let lower = condition.to_lowercase();

        // Try pattern: sector_N or sector N
        if let Some(start) = lower.find("sector") {
            let rest = &condition[start + 6..];
            let rest = rest.trim_start_matches(['_', ' ', '=']);

            // Extract the number/identifier
            let sector: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();

            if !sector.is_empty() {
                return Some(sector);
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_router::learning::LessonPattern;

    #[test]
    fn test_extract_sector() {
        assert_eq!(PolicyCache::extract_sector("sector_4"), Some("4".to_string()));
        assert_eq!(PolicyCache::extract_sector("sector 5"), Some("5".to_string()));
        assert_eq!(PolicyCache::extract_sector("IF sector_abc"), Some("abc".to_string()));
        assert_eq!(PolicyCache::extract_sector("no relevant data"), None);
    }

    #[test]
    fn test_policy_cache() {
        let mut cache = PolicyCache::new();
        assert!(cache.is_empty());

        let lesson = Lesson::new(
            "agent-1".to_string(),
            "swarm-1".to_string(),
            "navigation".to_string(),
            LessonPattern::new("sector_4".to_string(), "slow".to_string(), "avoid_crash".to_string()),
            0.8,
            3,
        );

        cache.add_lesson(lesson.clone());
        assert_eq!(cache.len(), 1);
        assert!(cache.should_slowdown("4").is_some());
        assert!(cache.should_avoid("4").is_none());
    }
}
