use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;
use tokio::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub passed: bool,
    pub total_tests: u32,
    pub passed_tests: u32,
    pub failed_tests: u32,
    pub output: String,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyResult {
    pub applied: bool,
    pub test_result: Option<TestResult>,
    pub error: Option<String>,
}

pub struct SolutionApplier {
    timeout_seconds: u64,
}

impl Default for SolutionApplier {
    fn default() -> Self {
        Self {
            timeout_seconds: 300,
        }
    }
}

impl SolutionApplier {
    pub fn new(timeout_seconds: u64) -> Self {
        Self { timeout_seconds }
    }

    pub async fn apply_and_test(
        &self,
        repo_path: &Path,
        solution: &str,
        test_command: Option<&str>,
    ) -> Result<ApplyResult> {
        let patch_applied = self.apply_patch(repo_path, solution).await?;

        if !patch_applied {
            return Ok(ApplyResult {
                applied: false,
                test_result: None,
                error: Some("Failed to apply patch".to_string()),
            });
        }

        let test_result = if let Some(cmd) = test_command {
            Some(self.run_tests(repo_path, cmd).await?)
        } else {
            None
        };

        Ok(ApplyResult {
            applied: true,
            test_result,
            error: None,
        })
    }

    async fn apply_patch(&self, repo_path: &Path, solution: &str) -> Result<bool> {
        let diff_content = self.extract_diff(solution)?;

        if diff_content.is_empty() {
            return Err(Error::InvalidInput(
                "No git diff found in solution".to_string(),
            ));
        }

        let patch_file = repo_path.join(".arkavo_solution.patch");
        fs::write(&patch_file, diff_content).await?;

        let output = Command::new("git")
            .arg("apply")
            .arg("--check")
            .arg(&patch_file)
            .current_dir(repo_path)
            .output()
            .map_err(|e| Error::External(format!("Failed to check patch: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::External(format!(
                "Patch validation failed: {}",
                stderr
            )));
        }

        let apply_output = Command::new("git")
            .arg("apply")
            .arg(&patch_file)
            .current_dir(repo_path)
            .output()
            .map_err(|e| Error::External(format!("Failed to apply patch: {}", e)))?;

        fs::remove_file(&patch_file).await.ok();

        if apply_output.status.success() {
            Ok(true)
        } else {
            let stderr = String::from_utf8_lossy(&apply_output.stderr);
            Err(Error::External(format!(
                "Failed to apply patch: {}",
                stderr
            )))
        }
    }

    fn extract_diff(&self, solution: &str) -> Result<String> {
        let mut in_diff = false;
        let mut diff_lines = Vec::new();

        for line in solution.lines() {
            if line.starts_with("diff --git") {
                in_diff = true;
            }

            if in_diff {
                diff_lines.push(line);
            }

            if line.starts_with("```") && in_diff && !diff_lines.is_empty() {
                break;
            }
        }

        if diff_lines.is_empty() {
            for line in solution.lines() {
                if line.starts_with("---") || line.starts_with("+++") || line.starts_with("@@") {
                    in_diff = true;
                }

                if in_diff {
                    diff_lines.push(line);
                }
            }
        }

        Ok(diff_lines.join("\n"))
    }

    async fn run_tests(&self, repo_path: &Path, test_command: &str) -> Result<TestResult> {
        let parts: Vec<&str> = test_command.split_whitespace().collect();

        if parts.is_empty() {
            return Err(Error::InvalidInput("Empty test command".to_string()));
        }

        let (cmd, args) = parts.split_first().unwrap();

        let output = tokio::time::timeout(
            tokio::time::Duration::from_secs(self.timeout_seconds),
            tokio::process::Command::new(cmd)
                .args(args)
                .current_dir(repo_path)
                .output(),
        )
        .await
        .map_err(|_| Error::Timeout(format!("Tests timed out after {}s", self.timeout_seconds)))?
        .map_err(|e| Error::External(format!("Failed to run tests: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let combined_output = format!("{}\n{}", stdout, stderr);

        let (passed, total, passed_count, failed_count) =
            self.parse_test_output(&combined_output, test_command);

        let error_message = if !passed {
            Some(self.extract_error_message(&combined_output))
        } else {
            None
        };

        Ok(TestResult {
            passed,
            total_tests: total,
            passed_tests: passed_count,
            failed_tests: failed_count,
            output: combined_output,
            error_message,
        })
    }

    fn parse_test_output(&self, output: &str, test_command: &str) -> (bool, u32, u32, u32) {
        if test_command.contains("pytest") {
            self.parse_pytest_output(output)
        } else if test_command.contains("cargo test") {
            self.parse_cargo_test_output(output)
        } else if test_command.contains("npm test") || test_command.contains("jest") {
            self.parse_jest_output(output)
        } else {
            (output.contains("PASS") || output.contains("OK"), 0, 0, 0)
        }
    }

    fn parse_pytest_output(&self, output: &str) -> (bool, u32, u32, u32) {
        let mut passed = 0u32;
        let mut failed = 0u32;

        for line in output.lines() {
            if line.contains("passed") || line.contains("failed") {
                if let Some(passed_str) = line.split_whitespace().find(|s| s.ends_with("passed"))
                    && let Ok(num) = passed_str.trim_end_matches("passed").parse::<u32>() {
                        passed = num;
                    }

                if let Some(failed_str) = line.split_whitespace().find(|s| s.ends_with("failed"))
                    && let Ok(num) = failed_str.trim_end_matches("failed").parse::<u32>() {
                        failed = num;
                    }
            }
        }

        let total = passed + failed;
        let all_passed = failed == 0 && total > 0;

        (all_passed, total, passed, failed)
    }

    fn parse_cargo_test_output(&self, output: &str) -> (bool, u32, u32, u32) {
        let mut passed = 0u32;
        let mut failed = 0u32;

        for line in output.lines() {
            if line.contains("test result:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                for (i, part) in parts.iter().enumerate() {
                    if *part == "passed;" && i > 0
                        && let Ok(num) = parts[i - 1].parse::<u32>() {
                            passed = num;
                        }
                    if *part == "failed;" && i > 0
                        && let Ok(num) = parts[i - 1].parse::<u32>() {
                            failed = num;
                        }
                }
            }
        }

        let total = passed + failed;
        let all_passed = output.contains("test result: ok");

        (all_passed, total, passed, failed)
    }

    fn parse_jest_output(&self, output: &str) -> (bool, u32, u32, u32) {
        let mut passed = 0u32;
        let mut failed = 0u32;

        for line in output.lines() {
            if line.contains("Tests:") {
                let parts: Vec<&str> = line.split(',').collect();
                for part in parts {
                    if part.contains("passed")
                        && let Some(num_str) = part.split_whitespace().next()
                            && let Ok(num) = num_str.parse::<u32>() {
                                passed = num;
                            }
                    if part.contains("failed")
                        && let Some(num_str) = part.split_whitespace().next()
                            && let Ok(num) = num_str.parse::<u32>() {
                                failed = num;
                            }
                }
            }
        }

        let total = passed + failed;
        let all_passed = failed == 0 && total > 0;

        (all_passed, total, passed, failed)
    }

    fn extract_error_message(&self, output: &str) -> String {
        let lines: Vec<&str> = output.lines().collect();

        for (i, line) in lines.iter().enumerate() {
            if line.contains("FAILED") || line.contains("ERROR") || line.contains("AssertionError")
            {
                let context_lines = &lines[i.saturating_sub(2)..std::cmp::min(i + 5, lines.len())];
                return context_lines.join("\n");
            }
        }

        if lines.len() > 20 {
            lines[lines.len() - 20..].join("\n")
        } else {
            output.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_diff() {
        let applier = SolutionApplier::default();

        let solution = r#"
Here is the fix:

```diff
diff --git a/src/main.py b/src/main.py
index abc123..def456 100644
--- a/src/main.py
+++ b/src/main.py
@@ -10,7 +10,7 @@
 def process(data):
-    return data.value
+    return data.value if data else None

```
"#;

        let diff = applier.extract_diff(solution).unwrap();
        assert!(diff.contains("diff --git"));
        assert!(diff.contains("src/main.py"));
        assert!(diff.contains("+    return data.value if data else None"));
    }

    #[test]
    fn test_parse_pytest_output() {
        let applier = SolutionApplier::default();

        let output = "test_example.py::test_one PASSED\ntest_example.py::test_two FAILED\n\n=== 5passed 5failed in 2.3s ===";
        let (passed, total, passed_count, failed_count) = applier.parse_pytest_output(output);

        assert!(!passed);
        assert_eq!(total, 10);
        assert_eq!(passed_count, 5);
        assert_eq!(failed_count, 5);
    }

    #[test]
    fn test_parse_cargo_test_output() {
        let applier = SolutionApplier::default();

        let output = "test result: ok. 15 passed; 0 failed; 0 ignored";
        let (passed, total, passed_count, failed_count) = applier.parse_cargo_test_output(output);

        assert!(passed);
        assert_eq!(total, 15);
        assert_eq!(passed_count, 15);
        assert_eq!(failed_count, 0);
    }
}
