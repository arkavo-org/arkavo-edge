//! Policy cache for fast lesson lookup by sector
//!
//! Indexes lessons by sector ID for behavior policy checks.

use std::collections::HashMap;

use arkavo_router::learning::Lesson;
use uuid::Uuid;

/// Cache of lessons indexed by sector for fast policy lookup
pub struct PolicyCache {
    /// Lessons indexed by sector ID
    lessons_by_sector: HashMap<String, Vec<Lesson>>,
    /// All lessons by ID for quick lookup
    lessons_by_id: HashMap<Uuid, Lesson>,
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
        }
    }

    /// Add a lesson to the cache
    pub fn add_lesson(&mut self, lesson: Lesson) {
        // Extract sector from condition (simple parsing)
        let sector_id = Self::extract_sector(&lesson.pattern.condition);

        if let Some(sector) = sector_id {
            self.lessons_by_sector
                .entry(sector)
                .or_default()
                .push(lesson.clone());
        }

        self.lessons_by_id.insert(lesson.id, lesson);
    }

    /// Check if there's a slowdown lesson for a sector
    pub fn should_slowdown(&self, sector_id: &str) -> Option<&Lesson> {
        self.lessons_by_sector.get(sector_id).and_then(|lessons| {
            lessons
                .iter()
                .find(|l| l.pattern.action.to_lowercase().contains("slow"))
        })
    }

    /// Check if there's an avoid lesson for a sector
    pub fn should_avoid(&self, sector_id: &str) -> Option<&Lesson> {
        self.lessons_by_sector.get(sector_id).and_then(|lessons| {
            lessons
                .iter()
                .find(|l| l.pattern.action.to_lowercase().contains("avoid"))
        })
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
