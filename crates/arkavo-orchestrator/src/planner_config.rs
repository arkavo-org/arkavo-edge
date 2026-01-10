//! Adaptive planner configuration based on model capability tier
//!
//! Provides different prompt strategies for small, medium, and large models
//! to optimize planning quality while respecting model constraints.

use crate::agent_assignment::AgentAssignment;
use arkavo_router::PlannerTier;

/// Configuration for adaptive planning prompts based on model capability
pub trait PlannerConfig: Send + Sync {
    /// Generate a planning prompt appropriate for the model's capability
    fn planning_prompt(&self, assignment: &AgentAssignment) -> String;

    /// Get the recommended max tokens for output (None uses model default)
    fn max_tokens(&self) -> Option<usize>;

    /// Get the model capability tier
    fn tier(&self) -> PlannerTier;
}

/// Small model planner (< 2B params): Simple, structured prompts with minimal context
pub struct SmallModelPlanner;

impl PlannerConfig for SmallModelPlanner {
    fn planning_prompt(&self, assignment: &AgentAssignment) -> String {
        format!(
            r#"Plan for: {}

Type: {:?}
Tech: {:?}

Output exactly:
STEP 1: [action]
COMMANDS: [command]
VERIFY: tests

STEP 2: [action]
COMMANDS: [command]
VERIFY: build

Max 3 steps."#,
            assignment.issue_title,
            assignment.routing_decision.analysis.issue_type,
            assignment.routing_decision.analysis.technologies
        )
    }

    fn max_tokens(&self) -> Option<usize> {
        Some(512)
    }

    fn tier(&self) -> PlannerTier {
        PlannerTier::Small
    }
}

/// Medium model planner (2-7B params): Balanced detail with truncated descriptions
pub struct MediumModelPlanner;

impl PlannerConfig for MediumModelPlanner {
    fn planning_prompt(&self, assignment: &AgentAssignment) -> String {
        let truncated_body = truncate_to_chars(&assignment.issue_body, 500);
        format!(
            r#"Create execution plan for GitHub issue:

Title: {}
Description: {}
Type: {:?}
Complexity: {:?}

Return 3-4 steps. Each step:
STEP N: [description]
COMMANDS: [comma-separated]
VERIFY: tests, linter, build, or file_constraint_400
CONFIDENCE: [0.0-1.0]"#,
            assignment.issue_title,
            truncated_body,
            assignment.routing_decision.analysis.issue_type,
            assignment.routing_decision.analysis.complexity
        )
    }

    fn max_tokens(&self) -> Option<usize> {
        Some(1024)
    }

    fn tier(&self) -> PlannerTier {
        PlannerTier::Medium
    }
}

/// Large model planner (> 7B params or cloud): Full detail prompt
pub struct LargeModelPlanner;

impl PlannerConfig for LargeModelPlanner {
    fn planning_prompt(&self, assignment: &AgentAssignment) -> String {
        format!(
            "Generate a detailed execution plan for this GitHub issue:\n\n\
            Title: {}\n\n\
            Description: {}\n\n\
            Type: {:?}\n\
            Complexity: {:?}\n\
            Technologies: {:?}\n\n\
            Return a structured plan with 3-5 concrete steps. For each step:\n\
            1. Brief description\n\
            2. Specific commands to execute (e.g., cargo test, cargo build, git commands)\n\
            3. Verification checks (tests, linter, build success)\n\n\
            Format each step as:\n\
            STEP N: [description]\n\
            COMMANDS: [comma-separated commands]\n\
            VERIFY: [comma-separated: tests, linter, build, or file_constraint_400]\n\
            CONFIDENCE: [0.0-1.0]",
            assignment.issue_title,
            assignment.issue_body,
            assignment.routing_decision.analysis.issue_type,
            assignment.routing_decision.analysis.complexity,
            assignment.routing_decision.analysis.technologies
        )
    }

    fn max_tokens(&self) -> Option<usize> {
        None
    }

    fn tier(&self) -> PlannerTier {
        PlannerTier::Large
    }
}

/// Get the appropriate planner config for a model's capability tier
pub fn get_planner_config(tier: PlannerTier) -> Box<dyn PlannerConfig> {
    match tier {
        PlannerTier::Small => Box::new(SmallModelPlanner),
        PlannerTier::Medium => Box::new(MediumModelPlanner),
        PlannerTier::Large => Box::new(LargeModelPlanner),
    }
}

/// Truncate text to a maximum number of characters, respecting UTF-8 boundaries
fn truncate_to_chars(s: &str, max_chars: usize) -> &str {
    if s.len() <= max_chars {
        return s;
    }

    let mut end_idx = 0;
    for (i, c) in s.char_indices() {
        if i >= max_chars {
            break;
        }
        end_idx = i + c.len_utf8();
    }
    &s[..end_idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_to_chars() {
        let text = "Hello, world!";
        assert_eq!(truncate_to_chars(text, 5), "Hello");
        assert_eq!(truncate_to_chars(text, 100), text);
    }

    #[test]
    fn test_truncate_utf8() {
        let text = "Hello \u{1F600} world"; // Contains emoji
        let truncated = truncate_to_chars(text, 10);
        assert!(truncated.len() <= 14); // Safe boundary
    }

    #[test]
    fn test_planner_config_tiers() {
        assert_eq!(SmallModelPlanner.tier(), PlannerTier::Small);
        assert_eq!(MediumModelPlanner.tier(), PlannerTier::Medium);
        assert_eq!(LargeModelPlanner.tier(), PlannerTier::Large);
    }

    #[test]
    fn test_max_tokens() {
        assert_eq!(SmallModelPlanner.max_tokens(), Some(512));
        assert_eq!(MediumModelPlanner.max_tokens(), Some(1024));
        assert_eq!(LargeModelPlanner.max_tokens(), None);
    }

    #[test]
    fn test_get_planner_config() {
        let small = get_planner_config(PlannerTier::Small);
        assert_eq!(small.tier(), PlannerTier::Small);

        let medium = get_planner_config(PlannerTier::Medium);
        assert_eq!(medium.tier(), PlannerTier::Medium);

        let large = get_planner_config(PlannerTier::Large);
        assert_eq!(large.tier(), PlannerTier::Large);
    }
}
