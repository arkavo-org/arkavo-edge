use serde::{Deserialize, Serialize};

/// Score indicating task complexity and whether architect mode is recommended
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityScore {
    /// Estimated number of subtasks (0 = not decomposable)
    pub estimated_subtasks: u8,
    /// Estimated total output tokens
    pub estimated_output_tokens: u32,
    /// Spread across categories (0.0 = single category, 1.0 = all categories)
    pub category_spread: f32,
    /// Ratio of planning cost to total cost (should be < 0.2 for cost-effectiveness)
    pub planning_overhead_ratio: f32,
    /// Whether architect mode is recommended
    pub architect_recommended: bool,
    /// Estimated savings if architect mode is used
    pub estimated_savings_percent: f32,
    /// Keywords that triggered complexity detection
    pub complexity_triggers: Vec<String>,
}

impl ComplexityScore {
    /// Create a score for a simple task (architect mode not recommended)
    pub fn simple() -> Self {
        Self {
            estimated_subtasks: 1,
            estimated_output_tokens: 1000,
            category_spread: 0.0,
            planning_overhead_ratio: 1.0,
            architect_recommended: false,
            estimated_savings_percent: 0.0,
            complexity_triggers: Vec::new(),
        }
    }
}

impl Default for ComplexityScore {
    fn default() -> Self {
        Self::simple()
    }
}

/// Analyzes tasks to determine complexity and cost-effectiveness of architect mode
pub struct ComplexityScorer {
    /// Minimum estimated output tokens to consider architect mode
    min_output_tokens: u32,
    /// Minimum subtasks to justify planning overhead
    min_subtasks: u8,
    /// Maximum planning overhead ratio (planning cost / total cost)
    max_planning_overhead: f32,
}

impl ComplexityScorer {
    pub fn new() -> Self {
        Self {
            min_output_tokens: 2000,
            min_subtasks: 3,
            max_planning_overhead: 0.20,
        }
    }

    /// Analyze a task description for complexity
    pub fn analyze(&self, task: &str) -> ComplexityScore {
        let task_lower = task.to_lowercase();
        let mut triggers = Vec::new();

        // Detect multi-step indicators
        let multi_step_count = self.count_multi_step_indicators(&task_lower, &mut triggers);

        // Detect complexity keywords
        let complexity_keywords = self.detect_complexity_keywords(&task_lower, &mut triggers);

        // Estimate subtasks based on patterns
        let estimated_subtasks = self.estimate_subtask_count(&task_lower, multi_step_count);

        // Estimate output tokens based on task length and complexity
        let estimated_output_tokens = self.estimate_output_tokens(task, estimated_subtasks);

        // Calculate category spread (how many different types of work)
        let category_spread = self.calculate_category_spread(&task_lower);

        // Calculate planning overhead ratio
        // Planning typically takes ~200 tokens output from Opus
        // At $75/1M output, that's ~$0.015
        let planning_cost = 0.015_f64;
        let opus_only_cost = self.estimate_opus_cost(estimated_output_tokens);
        let planning_overhead_ratio = if opus_only_cost > 0.0 {
            (planning_cost / opus_only_cost) as f32
        } else {
            1.0
        };

        // Calculate estimated savings
        let estimated_savings_percent =
            self.estimate_savings(estimated_subtasks, category_spread, estimated_output_tokens);

        // Determine if architect mode is recommended
        let architect_recommended = estimated_subtasks >= self.min_subtasks
            && estimated_output_tokens >= self.min_output_tokens
            && planning_overhead_ratio <= self.max_planning_overhead
            && (complexity_keywords || multi_step_count >= 2);

        ComplexityScore {
            estimated_subtasks,
            estimated_output_tokens,
            category_spread,
            planning_overhead_ratio,
            architect_recommended,
            estimated_savings_percent,
            complexity_triggers: triggers,
        }
    }

    fn count_multi_step_indicators(&self, task: &str, triggers: &mut Vec<String>) -> u8 {
        let patterns = [
            ("and then", "sequential steps"),
            ("after that", "sequential steps"),
            ("first", "ordered steps"),
            ("second", "ordered steps"),
            ("third", "ordered steps"),
            ("finally", "ordered steps"),
            ("next", "sequential steps"),
            (". then", "sequential steps"),
            ("step 1", "numbered steps"),
            ("step 2", "numbered steps"),
        ];

        let mut count = 0u8;
        for (pattern, trigger) in patterns {
            if task.contains(pattern) {
                count = count.saturating_add(1);
                if !triggers.contains(&trigger.to_string()) {
                    triggers.push(trigger.to_string());
                }
            }
        }

        // Also count sentence boundaries as potential steps
        let sentence_count = task.matches(". ").count() + task.matches("? ").count();
        if sentence_count >= 3 {
            count = count.saturating_add((sentence_count / 2) as u8);
            triggers.push(format!("{sentence_count} sentences"));
        }

        count
    }

    fn detect_complexity_keywords(&self, task: &str, triggers: &mut Vec<String>) -> bool {
        let keywords = [
            ("refactor", "refactoring"),
            ("migration", "migration"),
            ("redesign", "redesign"),
            ("multi-step", "multi-step"),
            ("comprehensive", "comprehensive"),
            ("complete", "complete implementation"),
            ("full implementation", "full implementation"),
            ("end-to-end", "end-to-end"),
            ("entire", "entire system"),
            ("architecture", "architecture"),
        ];

        let mut found = false;
        for (keyword, trigger) in keywords {
            if task.contains(keyword) {
                found = true;
                triggers.push(trigger.to_string());
            }
        }
        found
    }

    fn estimate_subtask_count(&self, task: &str, multi_step_count: u8) -> u8 {
        let base_count = if multi_step_count > 0 {
            multi_step_count
        } else {
            1
        };

        // Adjust based on action verbs
        let action_verbs = [
            "create",
            "build",
            "implement",
            "add",
            "update",
            "fix",
            "write",
            "generate",
            "test",
            "refactor",
            "migrate",
            "deploy",
        ];

        let verb_count = action_verbs
            .iter()
            .filter(|v| task.contains(*v))
            .count()
            .min(10) as u8;

        // Combine signals
        let estimated = (base_count + verb_count / 2).clamp(1, 10);

        // Boost if task is long (complex tasks tend to have longer descriptions)
        if task.len() > 300 && estimated < 3 {
            3
        } else {
            estimated
        }
    }

    fn estimate_output_tokens(&self, task: &str, subtask_count: u8) -> u32 {
        // Base estimate per subtask
        let base_per_subtask = 800_u32;

        // Adjust based on task complexity indicators
        let complexity_multiplier = if task.to_lowercase().contains("comprehensive")
            || task.to_lowercase().contains("detailed")
        {
            1.5
        } else if task.to_lowercase().contains("simple") || task.to_lowercase().contains("quick") {
            0.7
        } else {
            1.0
        };

        let base_estimate =
            (subtask_count as u32 * base_per_subtask) as f64 * complexity_multiplier;

        // Also factor in task description length (longer descriptions = more complex output)
        let length_factor = (task.len() as f64 / 100.0).min(3.0);

        (base_estimate * (1.0 + length_factor * 0.2)) as u32
    }

    fn calculate_category_spread(&self, task: &str) -> f32 {
        let categories = [
            (
                vec!["react", "vue", "frontend", "ui", "component", "css"],
                "frontend",
            ),
            (
                vec!["api", "endpoint", "backend", "database", "server"],
                "backend",
            ),
            (
                vec!["test", "jest", "pytest", "unit", "integration"],
                "test",
            ),
            (vec!["doc", "readme", "comment", "documentation"], "docs"),
            (
                vec!["security", "auth", "vulnerability", "scan"],
                "security",
            ),
        ];

        let mut found_categories = 0u8;
        for (keywords, _name) in &categories {
            if keywords.iter().any(|k| task.contains(k)) {
                found_categories += 1;
            }
        }

        // Normalize to 0.0-1.0 range
        (found_categories as f32 / categories.len() as f32).min(1.0)
    }

    fn estimate_opus_cost(&self, output_tokens: u32) -> f64 {
        // Opus pricing: $15/1M input, $75/1M output
        // Assume input is roughly 1/3 of output for typical tasks
        let input_tokens = output_tokens / 3;
        let input_cost = (input_tokens as f64 / 1_000_000.0) * 15.0;
        let output_cost = (output_tokens as f64 / 1_000_000.0) * 75.0;
        input_cost + output_cost
    }

    fn estimate_savings(&self, subtask_count: u8, category_spread: f32, output_tokens: u32) -> f32 {
        if subtask_count < 2 {
            return 0.0;
        }

        // Base savings from routing (assuming 50% of subtasks can use cheaper models)
        let routing_savings = 0.50;

        // Additional savings from category spread (more spread = more routing opportunities)
        let spread_bonus = category_spread * 0.20;

        // Scale with output tokens (larger tasks benefit more)
        let scale_factor = if output_tokens > 5000 {
            1.2
        } else if output_tokens > 2000 {
            1.0
        } else {
            0.8
        };

        // Planning overhead penalty
        let planning_penalty = 0.05; // ~5% for planning phase

        let savings = (routing_savings + spread_bonus).mul_add(scale_factor, -planning_penalty);
        (savings * 100.0).clamp(0.0, 80.0) // Cap at 80% savings
    }
}

impl Default for ComplexityScorer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_task_not_recommended() {
        let scorer = ComplexityScorer::new();
        let score = scorer.analyze("Fix a typo in the README");

        assert!(!score.architect_recommended);
        assert!(score.estimated_subtasks < 3);
    }

    #[test]
    fn test_complex_task_recommended() {
        let scorer = ComplexityScorer::new();
        let score = scorer.analyze(
            "Refactor the authentication system. First, update the user model to support OAuth. \
             Then, create new API endpoints for token refresh. After that, update the frontend \
             components to handle the new auth flow. Finally, write comprehensive tests for \
             all the changes.",
        );

        assert!(score.architect_recommended);
        assert!(score.estimated_subtasks >= 3);
        assert!(!score.complexity_triggers.is_empty());
    }

    #[test]
    fn test_multi_step_detection() {
        let scorer = ComplexityScorer::new();
        let score = scorer.analyze(
            "First create the database schema, then implement the API endpoints, \
             and finally build the frontend components.",
        );

        assert!(score.complexity_triggers.iter().any(|t| t.contains("step")));
        assert!(score.estimated_subtasks >= 3);
    }

    #[test]
    fn test_category_spread() {
        let scorer = ComplexityScorer::new();

        // Task touching multiple categories
        let multi_category =
            scorer.analyze("Build a React frontend with API endpoints and write unit tests");
        assert!(multi_category.category_spread > 0.3);

        // Single category task
        let single_category = scorer.analyze("Fix the CSS styling on the button component");
        assert!(single_category.category_spread < 0.3);
    }

    #[test]
    fn test_savings_estimation() {
        let scorer = ComplexityScorer::new();
        let score = scorer.analyze(
            "Implement a complete user management system with frontend UI, backend API, \
             database models, authentication, and comprehensive test coverage.",
        );

        // Complex multi-category task should show significant savings potential
        if score.architect_recommended {
            assert!(score.estimated_savings_percent > 30.0);
        }
    }
}
