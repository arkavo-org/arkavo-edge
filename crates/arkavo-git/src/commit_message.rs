use crate::attribution::format_commit_message;
use crate::{DiffOptions, GitManager, Result};
use git2::Repository;
use std::collections::HashMap;

/// Analyzes changes and generates an appropriate commit message
pub struct CommitMessageGenerator {
    git_manager: GitManager,
}

impl CommitMessageGenerator {
    pub fn new() -> Self {
        Self {
            git_manager: GitManager::new(),
        }
    }

    /// Generate a commit message based on the current changes
    pub fn generate(&self, repo: &Repository) -> Result<String> {
        let status = self.git_manager.status(repo)?;
        let diff_options = DiffOptions::default();
        let diff = self.git_manager.diff(repo, &diff_options)?;

        // Analyze the changes
        let change_summary = self.analyze_changes(&status, &diff);

        // Generate message based on changes
        let message = self.create_message(&change_summary);

        // Check if files are being modified
        let files_modified =
            !status.modified.is_empty() || !status.added.is_empty() || !status.deleted.is_empty();

        // Add attribution
        let formatted_message = format_commit_message(&message, files_modified);

        Ok(formatted_message)
    }

    fn analyze_changes(&self, status: &crate::GitStatus, diff: &str) -> ChangeSummary {
        let mut summary = ChangeSummary::default();

        // Count file types
        let all_files: Vec<&String> = status
            .modified
            .iter()
            .chain(status.added.iter())
            .chain(status.deleted.iter())
            .collect();

        for file in &all_files {
            if let Some(ext) = std::path::Path::new(file).extension() {
                let ext_str = ext.to_string_lossy().to_string();
                *summary.file_types.entry(ext_str).or_insert(0) += 1;
            }
        }

        // Analyze diff for common patterns
        let lines: Vec<&str> = diff.lines().collect();
        for line in &lines {
            if line.starts_with("+++") || line.starts_with("---") {
                continue;
            }

            // Count additions and deletions
            if line.starts_with('+') && !line.starts_with("+++") {
                summary.additions += 1;

                // Check for common patterns
                if line.contains("test") || line.contains("Test") {
                    summary.has_tests = true;
                }
                if line.contains("fn ") || line.contains("pub fn") {
                    summary.functions_added += 1;
                }
                if line.contains("TODO") || line.contains("FIXME") {
                    summary.has_todos = true;
                }
            } else if line.starts_with('-') && !line.starts_with("---") {
                summary.deletions += 1;
            }
        }

        // Determine primary change type
        summary.change_type = if status.added.len() > status.modified.len() + status.deleted.len() {
            ChangeType::Feature
        } else if status.deleted.len() > status.added.len() + status.modified.len() {
            ChangeType::Refactor
        } else if summary.has_tests {
            ChangeType::Test
        } else if all_files
            .iter()
            .any(|f| f.contains("README") || f.ends_with(".md"))
        {
            ChangeType::Docs
        } else if summary.additions < 50 && summary.deletions < 50 {
            ChangeType::Fix
        } else {
            ChangeType::Update
        };

        summary.total_files = all_files.len();
        summary.files_added = status.added.len();
        summary.files_modified = status.modified.len();
        summary.files_deleted = status.deleted.len();

        summary
    }

    fn create_message(&self, summary: &ChangeSummary) -> String {
        let prefix = match summary.change_type {
            ChangeType::Feature => "feat",
            ChangeType::Fix => "fix",
            ChangeType::Docs => "docs",
            ChangeType::Test => "test",
            ChangeType::Refactor => "refactor",
            ChangeType::Update => "update",
        };

        let main_action = match summary.change_type {
            ChangeType::Feature => {
                if summary.functions_added > 0 {
                    format!(
                        "add {} new function{}",
                        summary.functions_added,
                        if summary.functions_added > 1 { "s" } else { "" }
                    )
                } else {
                    "add new functionality".to_string()
                }
            }
            ChangeType::Fix => "fix issues".to_string(),
            ChangeType::Docs => "update documentation".to_string(),
            ChangeType::Test => {
                if summary.files_added > 0 {
                    "add tests".to_string()
                } else {
                    "update tests".to_string()
                }
            }
            ChangeType::Refactor => "refactor code".to_string(),
            ChangeType::Update => {
                let primary_type = summary
                    .file_types
                    .iter()
                    .max_by_key(|(_, count)| *count)
                    .map_or("files", |(ext, _)| ext.as_str());

                format!("update {primary_type} files")
            }
        };

        let details = format!(
            "{} file{} changed ({} added, {} modified, {} deleted)",
            summary.total_files,
            if summary.total_files == 1 { "" } else { "s" },
            summary.files_added,
            summary.files_modified,
            summary.files_deleted
        );

        format!("{prefix}: {main_action} - {details}")
    }
}

impl Default for CommitMessageGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Default, Debug)]
struct ChangeSummary {
    change_type: ChangeType,
    total_files: usize,
    files_added: usize,
    files_modified: usize,
    files_deleted: usize,
    additions: usize,
    deletions: usize,
    functions_added: usize,
    has_tests: bool,
    has_todos: bool,
    file_types: HashMap<String, usize>,
}

#[derive(Default, Debug)]
enum ChangeType {
    Feature,
    Fix,
    Docs,
    Test,
    Refactor,
    #[default]
    Update,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_commit_message_generation() {
        let temp_dir = TempDir::new().unwrap();
        let manager = GitManager::new();

        // Initialize repo
        manager.init_repo(temp_dir.path()).unwrap();
        let repo = manager.open_repo(temp_dir.path()).unwrap();

        // Set up git config for tests
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();

        // Create initial commit
        fs::write(temp_dir.path().join("main.rs"), "fn main() {}").unwrap();
        manager.add_all(&repo).unwrap();
        manager.commit_changes(&repo, "Initial commit").unwrap();

        // Make changes
        fs::write(temp_dir.path().join("lib.rs"), "pub fn new_function() {}").unwrap();
        fs::write(
            temp_dir.path().join("test.rs"),
            "#[test]\nfn test_something() {}",
        )
        .unwrap();

        let generator = CommitMessageGenerator::new();
        let message = generator.generate(&repo).unwrap();

        assert!(message.starts_with("feat:") || message.starts_with("test:"));
        assert!(message.contains("2 files changed"));
    }
}
