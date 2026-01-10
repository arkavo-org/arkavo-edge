use arkavo_orchestrator::{ModelInfo, TaskResult, TaskUI};
use async_trait::async_trait;
use std::io::{self, Write};

/// Terminal implementation of TaskUI
///
/// Provides formatted terminal output for task execution feedback
/// with support for interactive prompts.
pub struct TerminalUI {
    /// Whether to use colored output
    use_colors: bool,
}

impl Default for TerminalUI {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalUI {
    pub fn new() -> Self {
        Self { use_colors: true }
    }

    /// Create a TerminalUI with colors disabled
    #[allow(dead_code)]
    pub fn without_colors() -> Self {
        Self { use_colors: false }
    }

    fn print_colored(&self, prefix: &str, message: &str, color_code: &str) {
        if self.use_colors {
            println!("{color_code}{prefix}\x1b[0m {message}");
        } else {
            println!("{prefix} {message}");
        }
    }
}

#[async_trait]
impl TaskUI for TerminalUI {
    fn status(&self, message: &str) {
        println!("{message}");
    }

    fn progress(&self, message: &str, percentage: Option<u8>) {
        if let Some(pct) = percentage {
            println!("[{pct:3}%] {message}");
        } else {
            println!("{message}");
        }
    }

    fn error(&self, message: &str) {
        self.print_colored("Error:", message, "\x1b[31m");
    }

    fn warn(&self, message: &str) {
        self.print_colored("Warning:", message, "\x1b[33m");
    }

    fn success(&self, message: &str) {
        if self.use_colors {
            println!("\x1b[32mOK\x1b[0m {message}");
        } else {
            println!("OK {message}");
        }
    }

    fn section(&self, title: &str) {
        println!();
        println!("=== {title} ===");
        println!();
    }

    async fn confirm(&self, prompt: &str) -> bool {
        print!("{prompt} [y/N]: ");
        if io::stdout().flush().is_err() {
            return false;
        }

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            return false;
        }

        matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
    }

    async fn select(&self, prompt: &str, options: &[&str]) -> Option<usize> {
        println!("{prompt}");
        for (i, option) in options.iter().enumerate() {
            println!("  [{i}] {option}");
        }
        print!("Enter choice: ");
        if io::stdout().flush().is_err() {
            return None;
        }

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            return None;
        }

        input.trim().parse().ok().filter(|&i| i < options.len())
    }

    async fn input(&self, prompt: &str) -> Option<String> {
        print!("{prompt}: ");
        if io::stdout().flush().is_err() {
            return None;
        }

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            return None;
        }

        let trimmed = input.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    fn show_models(&self, local: &[ModelInfo], cloud: &[ModelInfo]) {
        if !local.is_empty() {
            println!("Local Models:");
            for model in local {
                let cap_str = match model.capability {
                    arkavo_orchestrator::ModelCapability::Small => "Small",
                    arkavo_orchestrator::ModelCapability::Medium => "Medium",
                    arkavo_orchestrator::ModelCapability::Large => "Large",
                };
                let size = model
                    .size_gb
                    .map(|s| format!(" - {s:.1} GB"))
                    .unwrap_or_default();
                if self.use_colors {
                    println!("  \x1b[32mOK\x1b[0m {} ({cap_str}){size}", model.name);
                } else {
                    println!("  OK {} ({cap_str}){size}", model.name);
                }
            }
            println!();
        }

        if !cloud.is_empty() {
            println!("Cloud Models:");
            for model in cloud {
                if self.use_colors {
                    println!("  \x1b[32mOK\x1b[0m {} ({})", model.name, model.model_id);
                } else {
                    println!("  OK {} ({})", model.name, model.model_id);
                }
            }
            println!();
        }
    }

    fn show_result(&self, result: &TaskResult) {
        println!();
        if result.success {
            if self.use_colors {
                println!("\x1b[32mTask completed:\x1b[0m {}", result.message);
            } else {
                println!("Task completed: {}", result.message);
            }
            if let Some(oid) = &result.commit_oid {
                println!("  Commit: {oid}");
            }
            if result.changes_count > 0 {
                println!("  Files changed: {}", result.changes_count);
            }
        } else {
            self.print_colored("Task failed:", &result.message, "\x1b[31m");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_orchestrator::ModelCapability;
    use std::path::PathBuf;

    #[test]
    fn test_terminal_ui_creation() {
        let ui = TerminalUI::new();
        assert!(ui.use_colors);

        let ui_no_color = TerminalUI::without_colors();
        assert!(!ui_no_color.use_colors);
    }

    #[test]
    fn test_show_models_empty() {
        let ui = TerminalUI::without_colors();
        // Should not panic with empty lists
        ui.show_models(&[], &[]);
    }

    #[test]
    fn test_show_result_success() {
        let ui = TerminalUI::without_colors();
        let result = TaskResult::success("Test completed")
            .with_commit("abc123".to_string())
            .with_changes(3);
        // Should not panic
        ui.show_result(&result);
    }

    #[test]
    fn test_show_result_failure() {
        let ui = TerminalUI::without_colors();
        let result = TaskResult::failure("Test failed");
        // Should not panic
        ui.show_result(&result);
    }
}
