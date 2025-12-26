//! Tool pattern observer for capturing successful tool call patterns
//!
//! Records successful tool calls and synthesizes generic Lesson objects
//! that can be gossiped to other agents using the existing learning infrastructure.

use arkavo_router::learning::{Lesson, LessonPattern, ToolCallFormat, sanitize_args};
use std::collections::HashMap;
use tracing::{debug, info};

/// Category for tool format lessons
pub(super) const TOOL_FORMAT_CATEGORY: &str = "tool_format";

/// Minimum successful calls before a pattern is ready for lesson synthesis
const DEFAULT_MIN_EVIDENCE: u32 = 2;

/// Internal tracking of tool call patterns
struct PatternTracker {
    tool_name: String,
    format: ToolCallFormat,
    example_args: serde_json::Value,
    success_count: u32,
    model_name: String,
}

/// Observer for capturing tool call patterns from successful executions
pub struct ToolPatternObserver {
    /// Patterns by (tool_name, format) tuple
    patterns: HashMap<(String, ToolCallFormat), PatternTracker>,
    /// Minimum evidence (successful calls) before pattern is ready
    min_evidence: u32,
    /// Model name for attribution
    model_name: String,
}

impl ToolPatternObserver {
    /// Create a new observer with default settings
    pub fn new(model_name: String) -> Self {
        Self {
            patterns: HashMap::new(),
            min_evidence: DEFAULT_MIN_EVIDENCE,
            model_name,
        }
    }

    /// Create a new observer with custom minimum evidence threshold
    pub fn with_min_evidence(model_name: String, min_evidence: u32) -> Self {
        Self {
            patterns: HashMap::new(),
            min_evidence,
            model_name,
        }
    }

    /// Update the model name (e.g., when model changes at runtime)
    pub fn set_model_name(&mut self, model_name: String) {
        self.model_name = model_name;
    }

    /// Record a successful tool call
    pub fn record_success(
        &mut self,
        tool_name: &str,
        format: ToolCallFormat,
        _raw_invocation: &str,
        args: &serde_json::Value,
    ) {
        let key = (tool_name.to_string(), format);

        if let Some(tracker) = self.patterns.get_mut(&key) {
            tracker.success_count = tracker.success_count.saturating_add(1);
            debug!(
                tool = tool_name,
                format = %format,
                count = tracker.success_count,
                "Tool pattern success count updated"
            );
        } else {
            let tracker = PatternTracker {
                tool_name: tool_name.to_string(),
                format,
                example_args: sanitize_args(args),
                success_count: 1,
                model_name: self.model_name.clone(),
            };
            info!(
                tool = tool_name,
                format = %format,
                "New tool call pattern recorded"
            );
            self.patterns.insert(key, tracker);
        }
    }

    /// Check if any patterns are ready for synthesis
    pub fn has_ready_patterns(&self) -> bool {
        self.patterns
            .values()
            .any(|p| p.success_count >= self.min_evidence)
    }

    /// Get patterns ready for lesson synthesis (above threshold)
    pub fn get_ready_tool_names(&self) -> Vec<(&str, ToolCallFormat)> {
        self.patterns
            .values()
            .filter(|p| p.success_count >= self.min_evidence)
            .map(|p| (p.tool_name.as_str(), p.format))
            .collect()
    }

    /// Synthesize a generic Lesson from a ready pattern
    pub fn synthesize_lesson(
        &self,
        tool_name: &str,
        format: ToolCallFormat,
        agent_id: &str,
        swarm_id: &str,
    ) -> Option<Lesson> {
        let key = (tool_name.to_string(), format);
        let tracker = self.patterns.get(&key)?;

        if tracker.success_count < self.min_evidence {
            return None;
        }

        // Calculate confidence based on success count (diminishing returns)
        let confidence = 1.0 - (1.0 / (tracker.success_count as f64 + 1.0));

        // Build metadata with tool-specific information
        let metadata = serde_json::json!({
            "format": format.to_string(),
            "tool_name": tracker.tool_name,
            "parameter_names": tracker.example_args.as_object()
                .map(|obj| obj.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default(),
            "example_args": tracker.example_args,
            "example_invocation": format.format_example(&tracker.tool_name, &tracker.example_args),
            "model_name": tracker.model_name,
            "success_count": tracker.success_count
        });

        // Create pattern with condition/action/outcome and metadata
        let pattern = LessonPattern::new(
            format!("tool_call:{}", tracker.tool_name),
            format!("use_{format}_format"),
            "successful_tool_invocation".to_string(),
        )
        .with_metadata(metadata);

        Some(Lesson::new(
            agent_id.to_string(),
            swarm_id.to_string(),
            TOOL_FORMAT_CATEGORY.to_string(),
            pattern,
            confidence,
            tracker.success_count,
        ))
    }

    /// Synthesize all ready lessons
    pub fn synthesize_all_lessons(&self, agent_id: &str, swarm_id: &str) -> Vec<Lesson> {
        self.patterns
            .values()
            .filter(|p| p.success_count >= self.min_evidence)
            .filter_map(|p| self.synthesize_lesson(&p.tool_name, p.format, agent_id, swarm_id))
            .collect()
    }

    /// Clear all patterns (e.g., after lesson propagation)
    pub fn clear_patterns(&mut self) {
        self.patterns.clear();
    }

    /// Remove a specific pattern after it's been propagated
    pub fn remove_pattern(&mut self, tool_name: &str, format: ToolCallFormat) {
        self.patterns.remove(&(tool_name.to_string(), format));
    }

    /// Get the number of tracked patterns
    pub fn pattern_count(&self) -> usize {
        self.patterns.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_success() {
        let mut observer = ToolPatternObserver::new("test-model".to_string());

        observer.record_success(
            "get_sector",
            ToolCallFormat::Fence,
            r#"```get_sector\n{"id": 4}\n```"#,
            &serde_json::json!({"id": 4}),
        );

        assert_eq!(observer.pattern_count(), 1);
    }

    #[test]
    fn test_record_multiple_successes() {
        let mut observer = ToolPatternObserver::new("test-model".to_string());

        for _ in 0..5 {
            observer.record_success(
                "get_sector",
                ToolCallFormat::Fence,
                r#"```get_sector\n{"id": 4}\n```"#,
                &serde_json::json!({"id": 4}),
            );
        }

        assert!(observer.has_ready_patterns());
    }

    #[test]
    fn test_ready_patterns_threshold() {
        let mut observer = ToolPatternObserver::with_min_evidence("test".to_string(), 3);

        // First tool: 2 successes (not ready)
        observer.record_success("tool_a", ToolCallFormat::Fence, "", &serde_json::json!({}));
        observer.record_success("tool_a", ToolCallFormat::Fence, "", &serde_json::json!({}));

        // Second tool: 3 successes (ready)
        for _ in 0..3 {
            observer.record_success("tool_b", ToolCallFormat::Json, "", &serde_json::json!({}));
        }

        let ready = observer.get_ready_tool_names();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].0, "tool_b");
    }

    #[test]
    fn test_lesson_synthesis() {
        let mut observer = ToolPatternObserver::with_min_evidence("test".to_string(), 2);

        observer.record_success(
            "get_sector",
            ToolCallFormat::Fence,
            "",
            &serde_json::json!({"id": 4}),
        );
        observer.record_success(
            "get_sector",
            ToolCallFormat::Fence,
            "",
            &serde_json::json!({"id": 4}),
        );

        let lesson = observer
            .synthesize_lesson("get_sector", ToolCallFormat::Fence, "agent-1", "swarm-1")
            .unwrap();

        assert_eq!(lesson.category, TOOL_FORMAT_CATEGORY);
        assert!(lesson.confidence > 0.0);
        assert!(!lesson.propagated);

        // Check metadata
        let metadata = lesson.pattern.metadata.as_ref().unwrap();
        assert_eq!(metadata["tool_name"], "get_sector");
        assert_eq!(metadata["format"], "fence");
    }

    #[test]
    fn test_synthesize_all_lessons() {
        let mut observer = ToolPatternObserver::with_min_evidence("test".to_string(), 2);

        // Two tools, both ready
        observer.record_success("tool_a", ToolCallFormat::Fence, "", &serde_json::json!({}));
        observer.record_success("tool_a", ToolCallFormat::Fence, "", &serde_json::json!({}));
        observer.record_success("tool_b", ToolCallFormat::Json, "", &serde_json::json!({}));
        observer.record_success("tool_b", ToolCallFormat::Json, "", &serde_json::json!({}));

        let lessons = observer.synthesize_all_lessons("agent-1", "swarm-1");
        assert_eq!(lessons.len(), 2);

        // All should have tool_format category
        for lesson in &lessons {
            assert_eq!(lesson.category, TOOL_FORMAT_CATEGORY);
        }
    }
}
