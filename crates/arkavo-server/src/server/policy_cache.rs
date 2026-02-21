//! Policy cache for fast lesson lookup by sector and category
//!
//! Indexes lessons by sector ID for behavior policy checks,
//! by category for tool format lessons, and tracks quality trends
//! per (agent, category) for lesson-informed prompting.

use std::collections::{HashMap, HashSet};

use arkavo_router::learning::Lesson;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::tool_pattern_observer::TOOL_FORMAT_CATEGORY;

const MAX_BEHAVIOR_LESSONS_PER_KEY: usize = 5;
const MAX_QUALITY_HISTORY: usize = 20;
const MAX_GUIDANCE_LESSONS: usize = 5;

/// Quality trend data for a specific (agent, category) pair
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityTrend {
    #[serde(rename = "agentId")]
    pub agent_id: String,
    pub category: String,
    pub scores: Vec<f64>,
}

/// Cache of lessons indexed by sector for fast policy lookup
pub struct PolicyCache {
    /// Lessons indexed by sector ID
    lessons_by_sector: HashMap<String, Vec<Lesson>>,
    /// All lessons by ID for quick lookup
    lessons_by_id: HashMap<Uuid, Lesson>,
    /// Tool format lessons indexed by tool name
    tool_format_lessons: HashMap<String, Lesson>,
    /// Behavior lessons indexed by (agent_id, category), ring buffer of max 5
    behavior_lessons: HashMap<(String, String), Vec<Lesson>>,
    /// Quality score history per (agent_id, category), ring buffer of max 20
    quality_history: HashMap<(String, String), Vec<f64>>,
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
            behavior_lessons: HashMap::new(),
            quality_history: HashMap::new(),
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

            // Index as behavior lesson by (agent_id, category)
            let key = (lesson.agent_id.clone(), lesson.category.clone());
            let entries = self.behavior_lessons.entry(key).or_default();
            if entries.len() >= MAX_BEHAVIOR_LESSONS_PER_KEY {
                entries.remove(0);
            }
            entries.push(lesson.clone());

            // Record quality from lesson metadata if present
            if let Some(meta) = &lesson.pattern.metadata
                && let Some(score) = meta.get("quality_score").and_then(|v| v.as_f64())
            {
                self.record_quality(&lesson.agent_id, &lesson.category, score);
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

    /// Record a quality score for an (agent, category) pair
    pub fn record_quality(&mut self, agent_id: &str, category: &str, score: f64) {
        let key = (agent_id.to_string(), category.to_string());
        let history = self.quality_history.entry(key).or_default();
        if history.len() >= MAX_QUALITY_HISTORY {
            history.remove(0);
        }
        history.push(score);
    }

    /// Get quality trend for a specific (agent, category) pair
    pub fn get_quality_trend(&self, agent_id: &str, category: &str) -> Option<&Vec<f64>> {
        let key = (agent_id.to_string(), category.to_string());
        self.quality_history.get(&key)
    }

    /// Get all quality trends across all (agent, category) pairs
    pub fn get_all_trends(&self) -> Vec<QualityTrend> {
        self.quality_history
            .iter()
            .map(|((agent_id, category), scores)| QualityTrend {
                agent_id: agent_id.clone(),
                category: category.clone(),
                scores: scores.clone(),
            })
            .collect()
    }

    /// Build behavior guidance text from cached non-tool_format lessons
    ///
    /// Optionally filters by category. Returns up to 5 lessons formatted
    /// as guidance for prompt injection.
    pub fn get_behavior_guidance(&self, category: Option<&str>) -> String {
        let mut seen_conditions = HashSet::new();
        let lessons: Vec<&Lesson> = self
            .behavior_lessons
            .iter()
            .filter(|((_, cat), _)| category.is_none() || category == Some(cat.as_str()))
            .flat_map(|(_, lessons)| lessons.iter())
            .filter(|l| seen_conditions.insert(l.pattern.condition.as_str()))
            .take(MAX_GUIDANCE_LESSONS)
            .collect();

        if lessons.is_empty() {
            return String::new();
        }

        let mut lines = vec!["Learned behavior guidance:".to_string()];
        for lesson in lessons {
            lines.push(format!(
                "- When: {} | Do: {} | Expect: {}",
                lesson.pattern.condition, lesson.pattern.action, lesson.pattern.expected_outcome
            ));
        }
        lines.join("\n")
    }

    /// Get behavior lesson count
    pub fn behavior_lesson_count(&self) -> usize {
        self.behavior_lessons.values().map(|v| v.len()).sum()
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
        assert_eq!(
            PolicyCache::extract_sector("sector_4"),
            Some("4".to_string())
        );
        assert_eq!(
            PolicyCache::extract_sector("sector 5"),
            Some("5".to_string())
        );
        assert_eq!(
            PolicyCache::extract_sector("IF sector_abc"),
            Some("abc".to_string())
        );
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
            LessonPattern::new(
                "sector_4".to_string(),
                "slow".to_string(),
                "avoid_crash".to_string(),
            ),
            0.8,
            3,
        );

        cache.add_lesson(lesson.clone());
        assert_eq!(cache.len(), 1);
        assert!(cache.should_slowdown("4").is_some());
        assert!(cache.should_avoid("4").is_none());
    }

    #[test]
    fn test_quality_tracking() {
        let mut cache = PolicyCache::new();
        cache.record_quality("agent-1", "code_review", 0.3);
        cache.record_quality("agent-1", "code_review", 0.5);
        cache.record_quality("agent-1", "code_review", 0.7);

        let trend = cache.get_quality_trend("agent-1", "code_review").unwrap();
        assert_eq!(trend.len(), 3);
        assert!((trend[0] - 0.3).abs() < f64::EPSILON);
        assert!((trend[2] - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn test_quality_eviction() {
        let mut cache = PolicyCache::new();
        for i in 0..25 {
            cache.record_quality("a", "cat", i as f64 / 25.0);
        }
        let trend = cache.get_quality_trend("a", "cat").unwrap();
        assert_eq!(trend.len(), super::MAX_QUALITY_HISTORY);
        // Oldest entries evicted; first remaining entry is index 5
        assert!((trend[0] - 5.0 / 25.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_quality_isolation() {
        let mut cache = PolicyCache::new();
        cache.record_quality("agent-1", "code", 0.5);
        cache.record_quality("agent-2", "code", 0.9);

        assert!(cache.get_quality_trend("agent-1", "code").unwrap()[0] < 0.6);
        assert!(cache.get_quality_trend("agent-2", "code").unwrap()[0] > 0.8);
        assert!(cache.get_quality_trend("agent-1", "other").is_none());
    }

    #[test]
    fn test_all_trends() {
        let mut cache = PolicyCache::new();
        cache.record_quality("a", "x", 0.5);
        cache.record_quality("b", "y", 0.8);

        let trends = cache.get_all_trends();
        assert_eq!(trends.len(), 2);
    }

    #[test]
    fn test_behavior_guidance_empty() {
        let cache = PolicyCache::new();
        assert!(cache.get_behavior_guidance(None).is_empty());
    }

    #[test]
    fn test_behavior_guidance_from_lessons() {
        let mut cache = PolicyCache::new();

        let lesson = Lesson::new(
            "agent-1".to_string(),
            "local".to_string(),
            "code_review".to_string(),
            LessonPattern::new(
                "Agent agent-1 returns generic non-answers".to_string(),
                "Reduce routing weight".to_string(),
                "Substantive response".to_string(),
            ),
            0.8,
            1,
        );
        cache.add_lesson(lesson);

        let guidance = cache.get_behavior_guidance(None);
        assert!(guidance.contains("Learned behavior guidance:"));
        assert!(guidance.contains("generic non-answers"));
        assert!(guidance.contains("Substantive response"));
    }

    #[test]
    fn test_behavior_guidance_category_filter() {
        let mut cache = PolicyCache::new();

        let l1 = Lesson::new(
            "a".to_string(),
            "s".to_string(),
            "code".to_string(),
            LessonPattern::new(
                "code issue".to_string(),
                "fix".to_string(),
                "ok".to_string(),
            ),
            0.8,
            1,
        );
        let l2 = Lesson::new(
            "a".to_string(),
            "s".to_string(),
            "security".to_string(),
            LessonPattern::new(
                "sec issue".to_string(),
                "patch".to_string(),
                "safe".to_string(),
            ),
            0.8,
            1,
        );
        cache.add_lesson(l1);
        cache.add_lesson(l2);

        let code_guidance = cache.get_behavior_guidance(Some("code"));
        assert!(code_guidance.contains("code issue"));
        assert!(!code_guidance.contains("sec issue"));
    }

    #[test]
    fn test_behavior_lesson_eviction() {
        let mut cache = PolicyCache::new();
        for i in 0..8 {
            let lesson = Lesson::new(
                "a".to_string(),
                "s".to_string(),
                "cat".to_string(),
                LessonPattern::new(
                    format!("issue-{i}"),
                    "action".to_string(),
                    "outcome".to_string(),
                ),
                0.8,
                1,
            );
            cache.add_lesson(lesson);
        }
        // Ring buffer should have evicted oldest, keeping max 5
        assert_eq!(
            cache.behavior_lesson_count(),
            super::MAX_BEHAVIOR_LESSONS_PER_KEY
        );
    }

    #[test]
    fn test_tool_format_excluded_from_behavior() {
        let mut cache = PolicyCache::new();

        let tool_lesson = Lesson::new(
            "a".to_string(),
            "s".to_string(),
            super::TOOL_FORMAT_CATEGORY.to_string(),
            LessonPattern::new(
                "tool cond".to_string(),
                "tool act".to_string(),
                "tool out".to_string(),
            )
            .with_metadata(serde_json::json!({"tool_name": "read_file"})),
            0.9,
            5,
        );
        cache.add_lesson(tool_lesson);

        // tool_format lessons should not appear in behavior guidance
        assert!(cache.get_behavior_guidance(None).is_empty());
        assert_eq!(cache.behavior_lesson_count(), 0);
    }

    #[test]
    fn test_behavior_lesson_evicts_oldest() {
        let mut cache = PolicyCache::new();
        for i in 0..6 {
            let lesson = Lesson::new(
                "a".to_string(),
                "s".to_string(),
                "cat".to_string(),
                LessonPattern::new(
                    format!("condition-{i}"),
                    "action".to_string(),
                    "outcome".to_string(),
                ),
                0.8,
                1,
            );
            cache.add_lesson(lesson);
        }
        // First lesson (condition-0) should be evicted, keeping condition-1 through condition-5
        let guidance = cache.get_behavior_guidance(Some("cat"));
        assert!(
            !guidance.contains("condition-0"),
            "oldest lesson should be evicted"
        );
        assert!(guidance.contains("condition-1"));
        assert!(guidance.contains("condition-5"));
    }

    #[test]
    fn test_behavior_guidance_deduplication() {
        let mut cache = PolicyCache::new();
        // Add 3 lessons with identical condition to the same (agent, category)
        for _ in 0..3 {
            let lesson = Lesson::new(
                "a".to_string(),
                "s".to_string(),
                "code".to_string(),
                LessonPattern::new(
                    "duplicate condition".to_string(),
                    "some action".to_string(),
                    "some outcome".to_string(),
                ),
                0.8,
                1,
            );
            cache.add_lesson(lesson);
        }
        let guidance = cache.get_behavior_guidance(Some("code"));
        // The condition should appear exactly once despite 3 lessons
        let count = guidance.matches("duplicate condition").count();
        assert_eq!(
            count, 1,
            "dedup should collapse identical conditions to one line"
        );
    }

    #[test]
    fn test_behavior_guidance_cross_agent_limit() {
        let mut cache = PolicyCache::new();
        // Add 3 lessons to agent-A/cat-X
        for i in 0..3 {
            let lesson = Lesson::new(
                "agent-A".to_string(),
                "s".to_string(),
                "cat-X".to_string(),
                LessonPattern::new(format!("cond-A-{i}"), "act".to_string(), "out".to_string()),
                0.8,
                1,
            );
            cache.add_lesson(lesson);
        }
        // Add 3 lessons to agent-B/cat-X
        for i in 0..3 {
            let lesson = Lesson::new(
                "agent-B".to_string(),
                "s".to_string(),
                "cat-X".to_string(),
                LessonPattern::new(format!("cond-B-{i}"), "act".to_string(), "out".to_string()),
                0.8,
                1,
            );
            cache.add_lesson(lesson);
        }
        let guidance = cache.get_behavior_guidance(None);
        // Count "- When:" lines (each lesson produces one)
        let line_count = guidance
            .lines()
            .filter(|l| l.starts_with("- When:"))
            .count();
        assert!(
            line_count <= super::MAX_GUIDANCE_LESSONS,
            "guidance should have at most {} lines, got {}",
            super::MAX_GUIDANCE_LESSONS,
            line_count
        );
    }

    #[test]
    fn test_quality_history_preserves_order() {
        let mut cache = PolicyCache::new();
        cache.record_quality("a", "cat", 0.1);
        cache.record_quality("a", "cat", 0.5);
        cache.record_quality("a", "cat", 0.9);

        let trend = cache.get_quality_trend("a", "cat").unwrap();
        assert_eq!(trend, &vec![0.1, 0.5, 0.9]);
    }

    #[test]
    fn test_quality_from_lesson_metadata() {
        let mut cache = PolicyCache::new();

        let lesson = Lesson::new(
            "agent-1".to_string(),
            "local".to_string(),
            "code".to_string(),
            LessonPattern::new("issue".to_string(), "fix".to_string(), "good".to_string())
                .with_metadata(serde_json::json!({"quality_score": 0.2})),
            0.8,
            1,
        );
        cache.add_lesson(lesson);

        let trend = cache.get_quality_trend("agent-1", "code").unwrap();
        assert_eq!(trend.len(), 1);
        assert!((trend[0] - 0.2).abs() < f64::EPSILON);
    }
}
