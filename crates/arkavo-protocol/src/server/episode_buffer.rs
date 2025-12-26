//! Episode buffer for accumulating observations before synthesis
//!
//! Buffers tool call observations and episodes until thresholds are met.

use std::collections::{HashMap, VecDeque};

use arkavo_router::learning::Episode;
use chrono::{DateTime, Utc};

/// A single observation from a tool call
#[derive(Debug, Clone)]
pub struct ToolObservation {
    /// Tool name that was called
    pub tool_name: String,
    /// Arguments passed to the tool
    pub args: serde_json::Value,
    /// Result from the tool
    pub result: String,
    /// Whether the call succeeded
    pub success: bool,
    /// Latency in milliseconds
    pub latency_ms: u64,
    /// When this observation was recorded
    pub timestamp: DateTime<Utc>,
}

/// Buffer for accumulating tool call observations before episode synthesis
pub struct EpisodeBuffer {
    /// Observations indexed by category
    observations: HashMap<String, VecDeque<ToolObservation>>,
    /// Episodes ready for lesson synthesis, indexed by category
    episodes: HashMap<String, Vec<Episode>>,
    /// Threshold for synthesizing an episode from observations
    observation_threshold: usize,
    /// Threshold for synthesizing a lesson from episodes
    episode_threshold: usize,
}

impl Default for EpisodeBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl EpisodeBuffer {
    /// Create a new episode buffer with default thresholds
    pub fn new() -> Self {
        Self {
            observations: HashMap::new(),
            episodes: HashMap::new(),
            observation_threshold: 3, // Synthesize episode after 3 observations
            episode_threshold: 3,     // Synthesize lesson after 3 episodes
        }
    }

    /// Create a new episode buffer with custom thresholds
    pub fn with_thresholds(observation_threshold: usize, episode_threshold: usize) -> Self {
        Self {
            observations: HashMap::new(),
            episodes: HashMap::new(),
            observation_threshold,
            episode_threshold,
        }
    }

    /// Add an observation to the buffer
    pub fn add_observation(&mut self, obs: ToolObservation) {
        // Infer category from tool name
        let category = Self::infer_category(&obs.tool_name);
        self.observations
            .entry(category)
            .or_default()
            .push_back(obs);
    }

    /// Check if any category has enough observations for episode synthesis
    pub fn ready_for_episode_synthesis(&self) -> Option<String> {
        for (category, obs) in &self.observations {
            if obs.len() >= self.observation_threshold {
                return Some(category.clone());
            }
            // Also trigger on failure for immediate learning
            if obs.back().is_some_and(|o| !o.success) {
                return Some(category.clone());
            }
        }
        None
    }

    /// Take observations for a category (consumes them)
    pub fn take_observations(&mut self, category: &str) -> Vec<ToolObservation> {
        self.observations
            .remove(category)
            .map(|v| v.into_iter().collect())
            .unwrap_or_default()
    }

    /// Add an episode to the buffer
    pub fn add_episode(&mut self, episode: Episode) {
        let category = episode.task_category.clone();
        self.episodes.entry(category).or_default().push(episode);
    }

    /// Check if any category has enough episodes for lesson synthesis
    pub fn ready_for_lesson_synthesis(&self) -> Option<String> {
        for (category, eps) in &self.episodes {
            if eps.len() >= self.episode_threshold {
                return Some(category.clone());
            }
        }
        None
    }

    /// Take episodes for a category (consumes them)
    pub fn take_episodes(&mut self, category: &str) -> Vec<Episode> {
        self.episodes.remove(category).unwrap_or_default()
    }

    /// Infer task category from tool name
    fn infer_category(tool_name: &str) -> String {
        let lower = tool_name.to_lowercase();
        if lower.contains("sector") || lower.contains("navigate") || lower.contains("move") {
            "navigation".to_string()
        } else if lower.contains("hazard") || lower.contains("danger") || lower.contains("crash") {
            "hazard_response".to_string()
        } else if lower.contains("build") || lower.contains("craft") || lower.contains("create") {
            "construction".to_string()
        } else {
            "general".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_infer_category() {
        assert_eq!(EpisodeBuffer::infer_category("navigate_sector"), "navigation");
        assert_eq!(EpisodeBuffer::infer_category("move_to"), "navigation");
        assert_eq!(EpisodeBuffer::infer_category("report_hazard"), "hazard_response");
        assert_eq!(EpisodeBuffer::infer_category("build_wall"), "construction");
        assert_eq!(EpisodeBuffer::infer_category("unknown_tool"), "general");
    }

    #[test]
    fn test_episode_buffer_thresholds() {
        let buffer = EpisodeBuffer::with_thresholds(5, 2);
        assert_eq!(buffer.observation_threshold, 5);
        assert_eq!(buffer.episode_threshold, 2);
    }

    #[test]
    fn test_add_observation() {
        let mut buffer = EpisodeBuffer::new();

        let obs = ToolObservation {
            tool_name: "navigate_to".to_string(),
            args: serde_json::json!({}),
            result: "ok".to_string(),
            success: true,
            latency_ms: 100,
            timestamp: Utc::now(),
        };

        buffer.add_observation(obs);
        assert!(buffer.ready_for_episode_synthesis().is_none());

        // Add more observations to hit threshold
        for _ in 0..2 {
            buffer.add_observation(ToolObservation {
                tool_name: "navigate_to".to_string(),
                args: serde_json::json!({}),
                result: "ok".to_string(),
                success: true,
                latency_ms: 100,
                timestamp: Utc::now(),
            });
        }

        assert_eq!(buffer.ready_for_episode_synthesis(), Some("navigation".to_string()));
    }
}
