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
    /// Links to the routing decision that led to this tool call
    pub decision_trace_id: Option<uuid::Uuid>,
    /// Position in a multi-step execution
    pub step_index: u16,
    /// Which model generated this tool call
    pub model_name: Option<String>,
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
        let tool_name = obs.tool_name.clone();
        let success = obs.success;
        let entry = self.observations.entry(category.clone()).or_default();
        entry.push_back(obs);

        tracing::debug!(
            category = %category,
            tool_name = %tool_name,
            success = success,
            buffer_size = entry.len(),
            threshold = self.observation_threshold,
            "Observation buffered"
        );
    }

    /// Check if any category has enough observations for episode synthesis
    pub fn ready_for_episode_synthesis(&self) -> Option<String> {
        for (category, obs) in &self.observations {
            if obs.len() >= self.observation_threshold {
                tracing::info!(
                    category = %category,
                    observation_count = obs.len(),
                    threshold = self.observation_threshold,
                    "Observation threshold reached - ready for episode synthesis"
                );
                return Some(category.clone());
            }
            // Also trigger on failure for immediate learning
            if obs.back().is_some_and(|o| !o.success) {
                tracing::info!(
                    category = %category,
                    observation_count = obs.len(),
                    "Failure detected - triggering immediate episode synthesis"
                );
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
        let success = episode.outcome.success;
        let entry = self.episodes.entry(category.clone()).or_default();
        entry.push(episode);

        tracing::debug!(
            category = %category,
            success = success,
            episode_count = entry.len(),
            threshold = self.episode_threshold,
            "Episode added to buffer"
        );
    }

    /// Check if any category has enough episodes for lesson synthesis
    pub fn ready_for_lesson_synthesis(&self) -> Option<String> {
        for (category, eps) in &self.episodes {
            if eps.len() >= self.episode_threshold {
                tracing::info!(
                    category = %category,
                    episode_count = eps.len(),
                    threshold = self.episode_threshold,
                    "Episode threshold reached - ready for lesson synthesis"
                );
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
    use arkavo_router::learning::{EpisodeOutcome, Observation, QualityMetrics};

    fn obs(tool: &str, success: bool) -> ToolObservation {
        ToolObservation {
            tool_name: tool.to_string(),
            args: serde_json::json!({}),
            result: "ok".to_string(),
            success,
            latency_ms: 100,
            timestamp: Utc::now(),
            decision_trace_id: None,
            step_index: 0,
            model_name: None,
        }
    }

    fn episode(category: &str, success: bool) -> Episode {
        Episode::new(
            "agent-1".to_string(),
            "swarm-1".to_string(),
            uuid::Uuid::new_v4(),
            category.to_string(),
            Observation::new(
                serde_json::json!({}),
                "action".to_string(),
                serde_json::json!({}),
                vec![],
            ),
            EpisodeOutcome::new(success, QualityMetrics::new(0.8, 0.8, 0.8), 100, 0.0),
        )
    }

    #[test]
    fn test_infer_category() {
        assert_eq!(
            EpisodeBuffer::infer_category("navigate_sector"),
            "navigation"
        );
        assert_eq!(EpisodeBuffer::infer_category("move_to"), "navigation");
        assert_eq!(
            EpisodeBuffer::infer_category("report_hazard"),
            "hazard_response"
        );
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

        buffer.add_observation(obs("navigate_to", true));
        assert!(buffer.ready_for_episode_synthesis().is_none());

        // Add more observations to hit threshold
        for _ in 0..2 {
            buffer.add_observation(obs("navigate_to", true));
        }

        assert_eq!(
            buffer.ready_for_episode_synthesis(),
            Some("navigation".to_string())
        );
    }

    #[test]
    fn test_failure_triggers_immediate_synthesis() {
        let mut buffer = EpisodeBuffer::new();
        // A single failed observation should trigger synthesis immediately
        buffer.add_observation(obs("navigate_to", false));
        assert_eq!(
            buffer.ready_for_episode_synthesis(),
            Some("navigation".to_string())
        );
    }

    #[test]
    fn test_failure_after_success_triggers_synthesis() {
        let mut buffer = EpisodeBuffer::new();
        buffer.add_observation(obs("build_wall", true));
        // Not ready yet (1 success, threshold=3)
        assert!(buffer.ready_for_episode_synthesis().is_none());
        // Failure triggers immediately regardless of count
        buffer.add_observation(obs("build_wall", false));
        assert_eq!(
            buffer.ready_for_episode_synthesis(),
            Some("construction".to_string())
        );
    }

    #[test]
    fn test_multiple_categories_independent() {
        let mut buffer = EpisodeBuffer::with_thresholds(2, 2);
        buffer.add_observation(obs("navigate_to", true));
        buffer.add_observation(obs("build_wall", true));
        // Neither category has hit threshold of 2
        assert!(buffer.ready_for_episode_synthesis().is_none());

        // Push navigation over threshold
        buffer.add_observation(obs("move_forward", true));
        assert_eq!(
            buffer.ready_for_episode_synthesis(),
            Some("navigation".to_string())
        );
    }

    #[test]
    fn test_take_observations_consumes() {
        let mut buffer = EpisodeBuffer::new();
        for _ in 0..3 {
            buffer.add_observation(obs("navigate_to", true));
        }
        assert!(buffer.ready_for_episode_synthesis().is_some());

        let taken = buffer.take_observations("navigation");
        assert_eq!(taken.len(), 3);

        // After taking, buffer is empty for that category
        assert!(buffer.ready_for_episode_synthesis().is_none());
        // Taking again returns empty vec
        assert!(buffer.take_observations("navigation").is_empty());
    }

    #[test]
    fn test_episode_lesson_threshold() {
        let mut buffer = EpisodeBuffer::with_thresholds(3, 2);
        buffer.add_episode(episode("code", true));
        assert!(buffer.ready_for_lesson_synthesis().is_none());

        buffer.add_episode(episode("code", false));
        assert_eq!(
            buffer.ready_for_lesson_synthesis(),
            Some("code".to_string())
        );
    }

    #[test]
    fn test_take_episodes_consumes() {
        let mut buffer = EpisodeBuffer::with_thresholds(3, 2);
        buffer.add_episode(episode("code", true));
        buffer.add_episode(episode("code", true));
        assert!(buffer.ready_for_lesson_synthesis().is_some());

        let taken = buffer.take_episodes("code");
        assert_eq!(taken.len(), 2);

        assert!(buffer.ready_for_lesson_synthesis().is_none());
        assert!(buffer.take_episodes("code").is_empty());
    }

    #[test]
    fn test_episodes_multiple_categories_independent() {
        let mut buffer = EpisodeBuffer::with_thresholds(3, 2);
        buffer.add_episode(episode("code", true));
        buffer.add_episode(episode("security", true));
        // Neither at threshold
        assert!(buffer.ready_for_lesson_synthesis().is_none());

        buffer.add_episode(episode("code", true));
        assert_eq!(
            buffer.ready_for_lesson_synthesis(),
            Some("code".to_string())
        );
    }

    #[test]
    fn test_take_observations_preserves_order() {
        let mut buffer = EpisodeBuffer::with_thresholds(3, 3);
        for i in 0..3 {
            buffer.add_observation(ToolObservation {
                tool_name: "navigate_to".to_string(),
                args: serde_json::json!({}),
                result: format!("result-{i}"),
                success: true,
                latency_ms: (i + 1) as u64 * 100,
                timestamp: Utc::now(),
                decision_trace_id: None,
                step_index: 0,
                model_name: None,
            });
        }
        let taken = buffer.take_observations("navigation");
        assert_eq!(taken[0].result, "result-0");
        assert_eq!(taken[1].result, "result-1");
        assert_eq!(taken[2].result, "result-2");
        assert_eq!(taken[0].latency_ms, 100);
        assert_eq!(taken[2].latency_ms, 300);
    }
}
