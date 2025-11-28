use crate::server::{Tool, ToolSchema};
use crate::{Result, ToolError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::time::Instant;
use tokio::process::Command;

/// Severity levels for code review findings
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
}

impl Severity {
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "critical" | "error" => Self::Critical,
            "high" | "warning" => Self::High,
            "medium" | "info" => Self::Medium,
            _ => Self::Low,
        }
    }
}

/// A single code review finding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub severity: Severity,
    pub category: String,
    pub message: String,
    pub file: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub suggestion: Option<String>,
    pub analyzer: String,
    pub rule_id: Option<String>,
}

/// Summary of review results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSummary {
    pub total: usize,
    pub by_severity: HashMap<String, usize>,
    pub files_analyzed: usize,
    pub duration_ms: u64,
}

/// Code review tool orchestrating multiple analyzers
pub struct CodeReviewTool {
    schema: ToolSchema,
}

impl CodeReviewTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "code_review".to_string(),
                aliases: Some(vec![
                    "review".to_string(),
                    "analyze".to_string(),
                    "lint".to_string(),
                ]),
                description: "Automated code review with prioritized findings. Orchestrates \
                    Semgrep (if available), Clippy (for Rust), and built-in heuristics."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to analyze (default: current directory)"
                        },
                        "scope": {
                            "type": "string",
                            "enum": ["all", "changed", "staged"],
                            "description": "What to analyze: 'all' files, 'changed' (git diff), or 'staged' (default: changed)"
                        },
                        "severity_threshold": {
                            "type": "string",
                            "enum": ["critical", "high", "medium", "low"],
                            "description": "Minimum severity to report (default: medium)"
                        },
                        "max_findings": {
                            "type": "integer",
                            "description": "Maximum findings to return (default: 50)"
                        }
                    },
                    "required": []
                }),
            },
        }
    }

    /// Check if a binary is available in PATH
    fn is_binary_available(name: &str) -> bool {
        std::process::Command::new(name)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Check if the path is a Rust project
    fn is_rust_project(path: &str) -> bool {
        Path::new(path).join("Cargo.toml").exists() || Path::new("Cargo.toml").exists()
    }

    /// Get list of changed files from git
    async fn get_git_changed_files(&self, path: &str) -> Result<Vec<String>> {
        let output = Command::new("git")
            .args(["diff", "--name-only", "HEAD"])
            .current_dir(path)
            .output()
            .await
            .map_err(|e| ToolError::Mcp(format!("Failed to get git diff: {}", e)))?;

        if !output.status.success() {
            return Ok(Vec::new()); // Not a git repo or no changes
        }

        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect())
    }

    /// Get list of staged files from git
    async fn get_git_staged_files(&self, path: &str) -> Result<Vec<String>> {
        let output = Command::new("git")
            .args(["diff", "--name-only", "--cached"])
            .current_dir(path)
            .output()
            .await
            .map_err(|e| ToolError::Mcp(format!("Failed to get staged files: {}", e)))?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect())
    }

    /// Run Semgrep analysis
    async fn run_semgrep(&self, path: &str, files: &[String]) -> Vec<Finding> {
        if !Self::is_binary_available("semgrep") {
            return Vec::new();
        }

        let mut cmd = Command::new("semgrep");
        cmd.args(["--json", "--config", "auto"]);

        if files.is_empty() {
            cmd.arg(path);
        } else {
            for file in files {
                cmd.arg(file);
            }
        }

        cmd.current_dir(path);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());

        let output = match cmd.output().await {
            Ok(o) => o,
            Err(_) => return Vec::new(),
        };

        if !output.status.success() {
            return Vec::new();
        }

        self.parse_semgrep_output(&output.stdout)
    }

    /// Parse Semgrep JSON output
    fn parse_semgrep_output(&self, output: &[u8]) -> Vec<Finding> {
        let json: Value = match serde_json::from_slice(output) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };

        let results = match json.get("results").and_then(|r| r.as_array()) {
            Some(r) => r,
            None => return Vec::new(),
        };

        results
            .iter()
            .map(|r| {
                let severity = r
                    .get("extra")
                    .and_then(|e| e.get("severity"))
                    .and_then(|s| s.as_str())
                    .map(Severity::from_str)
                    .unwrap_or(Severity::Medium);

                Finding {
                    severity,
                    category: "security".to_string(),
                    message: r
                        .get("extra")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("Semgrep finding")
                        .to_string(),
                    file: r
                        .get("path")
                        .and_then(|p| p.as_str())
                        .unwrap_or("")
                        .to_string(),
                    line: r
                        .get("start")
                        .and_then(|s| s.get("line"))
                        .and_then(|l| l.as_u64())
                        .map(|l| l as u32),
                    column: r
                        .get("start")
                        .and_then(|s| s.get("col"))
                        .and_then(|c| c.as_u64())
                        .map(|c| c as u32),
                    suggestion: r
                        .get("extra")
                        .and_then(|e| e.get("fix"))
                        .and_then(|f| f.as_str())
                        .map(|s| s.to_string()),
                    analyzer: "semgrep".to_string(),
                    rule_id: r
                        .get("check_id")
                        .and_then(|c| c.as_str())
                        .map(|s| s.to_string()),
                }
            })
            .collect()
    }

    /// Run Clippy for Rust projects
    async fn run_clippy(&self, path: &str) -> Vec<Finding> {
        if !Self::is_binary_available("cargo") {
            return Vec::new();
        }

        let output = Command::new("cargo")
            .args(["clippy", "--message-format=json", "--", "-W", "clippy::all"])
            .current_dir(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;

        let output = match output {
            Ok(o) => o,
            Err(_) => return Vec::new(),
        };

        // Clippy outputs to stderr in JSON format (one message per line)
        self.parse_clippy_output(&output.stdout)
    }

    /// Parse Clippy JSON output
    fn parse_clippy_output(&self, output: &[u8]) -> Vec<Finding> {
        let output_str = String::from_utf8_lossy(output);
        let mut findings = Vec::new();

        for line in output_str.lines() {
            let json: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Only process compiler messages
            if json.get("reason").and_then(|r| r.as_str()) != Some("compiler-message") {
                continue;
            }

            let message = match json.get("message") {
                Some(m) => m,
                None => continue,
            };

            let level = message
                .get("level")
                .and_then(|l| l.as_str())
                .unwrap_or("warning");

            // Skip notes and help messages
            if level == "note" || level == "help" {
                continue;
            }

            let severity = Severity::from_str(level);

            let spans = message.get("spans").and_then(|s| s.as_array());
            let primary_span = spans
                .and_then(|s| {
                    s.iter().find(|span| {
                        span.get("is_primary")
                            .and_then(|p| p.as_bool())
                            .unwrap_or(false)
                    })
                })
                .or_else(|| spans.and_then(|s| s.first()));

            let file = primary_span
                .and_then(|s| s.get("file_name"))
                .and_then(|f| f.as_str())
                .unwrap_or("")
                .to_string();

            let line = primary_span
                .and_then(|s| s.get("line_start"))
                .and_then(|l| l.as_u64())
                .map(|l| l as u32);

            let column = primary_span
                .and_then(|s| s.get("column_start"))
                .and_then(|c| c.as_u64())
                .map(|c| c as u32);

            let suggestion = primary_span
                .and_then(|s| s.get("suggested_replacement"))
                .and_then(|r| r.as_str())
                .map(|s| s.to_string());

            let code = message
                .get("code")
                .and_then(|c| c.get("code"))
                .and_then(|c| c.as_str());

            let msg = message
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Clippy finding")
                .to_string();

            findings.push(Finding {
                severity,
                category: "lint".to_string(),
                message: msg,
                file,
                line,
                column,
                suggestion,
                analyzer: "clippy".to_string(),
                rule_id: code.map(|s| s.to_string()),
            });
        }

        findings
    }

    /// Run built-in heuristic checks
    async fn run_heuristics(&self, path: &str, files: &[String]) -> Vec<Finding> {
        let mut findings = Vec::new();

        for file in files {
            let file_path = Path::new(path).join(file);
            if !file_path.exists() || !file_path.is_file() {
                continue;
            }

            let content = match tokio::fs::read_to_string(&file_path).await {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Check for hardcoded secrets
            findings.extend(self.check_secrets(file, &content));

            // Check for long functions (Rust)
            if file.ends_with(".rs") {
                findings.extend(self.check_long_functions(file, &content));
            }
        }

        findings
    }

    /// Check for potential hardcoded secrets
    fn check_secrets(&self, file: &str, content: &str) -> Vec<Finding> {
        let mut findings = Vec::new();

        let patterns: &[(&str, &str)] = &[
            (
                r#"(?i)(api[_-]?key|apikey)\s*[=:]\s*["'][^"']{10,}["']"#,
                "Potential API key",
            ),
            (
                r#"(?i)(secret|password|passwd|pwd)\s*[=:]\s*["'][^"']{6,}["']"#,
                "Potential secret/password",
            ),
            (
                r"(?i)bearer\s+[a-zA-Z0-9._-]{20,}",
                "Potential bearer token",
            ),
            (
                r#"(?i)(aws[_-]?secret|aws[_-]?key)\s*[=:]\s*["'][^"']+["']"#,
                "Potential AWS credential",
            ),
            (r"ghp_[a-zA-Z0-9]{36}", "GitHub personal access token"),
            (r"sk-[a-zA-Z0-9]{48}", "Potential OpenAI API key"),
        ];

        for (line_num, line) in content.lines().enumerate() {
            for (pattern, msg) in patterns {
                if let Ok(re) = regex::Regex::new(pattern)
                    && re.is_match(line)
                {
                    findings.push(Finding {
                        severity: Severity::Critical,
                        category: "security".to_string(),
                        message: (*msg).to_string(),
                        file: file.to_string(),
                        line: Some((line_num + 1) as u32),
                        column: None,
                        suggestion: Some(
                            "Remove hardcoded secret and use environment variables".to_string(),
                        ),
                        analyzer: "heuristics".to_string(),
                        rule_id: Some("secret-detection".to_string()),
                    });
                    break; // Only one finding per line
                }
            }
        }

        findings
    }

    /// Check for long functions (>50 lines)
    fn check_long_functions(&self, file: &str, content: &str) -> Vec<Finding> {
        let mut findings = Vec::new();
        let mut in_function = false;
        let mut function_start = 0;
        let mut function_name = String::new();
        let mut brace_depth = 0;

        for (line_num, line) in content.lines().enumerate() {
            let trimmed = line.trim();

            // Detect function start
            if !in_function
                && (trimmed.starts_with("fn ")
                    || trimmed.starts_with("pub fn ")
                    || trimmed.starts_with("async fn ")
                    || trimmed.starts_with("pub async fn "))
            {
                // Extract function name
                if let Some(name_start) = trimmed.find("fn ") {
                    let after_fn = &trimmed[name_start + 3..];
                    if let Some(paren) = after_fn.find('(') {
                        function_name = after_fn[..paren].trim().to_string();
                    }
                }
                function_start = line_num + 1;
                in_function = true;
                brace_depth = 0;
            }

            if in_function {
                let open_braces = line.chars().filter(|&c| c == '{').count();
                let close_braces = line.chars().filter(|&c| c == '}').count();
                brace_depth += i32::try_from(open_braces).unwrap_or(0);
                brace_depth -= i32::try_from(close_braces).unwrap_or(0);

                if brace_depth <= 0 && line.contains('}') {
                    let function_length = line_num + 1 - function_start;
                    if function_length > 50 {
                        findings.push(Finding {
                            severity: Severity::Medium,
                            category: "maintainability".to_string(),
                            message: format!(
                                "Function '{}' is {} lines long (>50 lines)",
                                function_name, function_length
                            ),
                            file: file.to_string(),
                            line: Some(function_start as u32),
                            column: None,
                            suggestion: Some(
                                "Consider breaking this function into smaller functions"
                                    .to_string(),
                            ),
                            analyzer: "heuristics".to_string(),
                            rule_id: Some("long-function".to_string()),
                        });
                    }
                    in_function = false;
                }
            }
        }

        findings
    }

    /// Prioritize findings (security first, then by severity)
    fn prioritize_findings(&self, mut findings: Vec<Finding>) -> Vec<Finding> {
        findings.sort_by(|a, b| {
            // Security findings first
            let a_security = a.category == "security";
            let b_security = b.category == "security";

            match (a_security, b_security) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.severity.cmp(&b.severity),
            }
        });

        findings
    }
}

impl Default for CodeReviewTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for CodeReviewTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let start = Instant::now();

        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let scope = params
            .get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("changed");

        let severity_threshold = params
            .get("severity_threshold")
            .and_then(|v| v.as_str())
            .map(Severity::from_str)
            .unwrap_or(Severity::Medium);

        let max_findings = params
            .get("max_findings")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(50);

        // Get files to analyze
        let files = match scope {
            "changed" => self.get_git_changed_files(path).await?,
            "staged" => self.get_git_staged_files(path).await?,
            "all" => Vec::new(), // Empty means analyze all
            _ => self.get_git_changed_files(path).await?,
        };

        let files_count = if files.is_empty() { 1 } else { files.len() };

        let mut all_findings = Vec::new();

        // Run Semgrep
        all_findings.extend(self.run_semgrep(path, &files).await);

        // Run Clippy for Rust projects
        if Self::is_rust_project(path) {
            all_findings.extend(self.run_clippy(path).await);
        }

        // Run heuristics
        if !files.is_empty() {
            all_findings.extend(self.run_heuristics(path, &files).await);
        }

        // Filter by severity
        let filtered: Vec<Finding> = all_findings
            .into_iter()
            .filter(|f| f.severity <= severity_threshold)
            .collect();

        // Prioritize and limit
        let prioritized = self.prioritize_findings(filtered);
        let limited: Vec<Finding> = prioritized.into_iter().take(max_findings).collect();

        // Build summary
        let mut by_severity: HashMap<String, usize> = HashMap::new();
        for finding in &limited {
            let key = format!("{:?}", finding.severity).to_lowercase();
            *by_severity.entry(key).or_insert(0) += 1;
        }

        let summary = ReviewSummary {
            total: limited.len(),
            by_severity,
            files_analyzed: files_count,
            duration_ms: start.elapsed().as_millis() as u64,
        };

        Ok(json!({
            "findings": limited,
            "summary": summary
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema() {
        let tool = CodeReviewTool::new();
        let schema = tool.schema();

        assert_eq!(schema.name, "code_review");
        assert!(
            schema
                .aliases
                .as_ref()
                .unwrap()
                .contains(&"review".to_string())
        );
        assert!(schema.description.contains("Semgrep"));
    }

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Critical < Severity::High);
        assert!(Severity::High < Severity::Medium);
        assert!(Severity::Medium < Severity::Low);
    }

    #[test]
    fn test_severity_from_str() {
        assert_eq!(Severity::from_str("critical"), Severity::Critical);
        assert_eq!(Severity::from_str("error"), Severity::Critical);
        assert_eq!(Severity::from_str("warning"), Severity::High);
        assert_eq!(Severity::from_str("info"), Severity::Medium);
        assert_eq!(Severity::from_str("hint"), Severity::Low);
    }

    #[test]
    fn test_check_secrets() {
        let tool = CodeReviewTool::new();

        // Should detect API key
        let content = r#"API_KEY = "sk-1234567890abcdef1234567890abcdef12345678901234567890""#;
        let findings = tool.check_secrets("test.py", content);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].severity, Severity::Critical);

        // Should detect GitHub token
        let content = r#"token = "ghp_1234567890123456789012345678901234567890""#;
        let findings = tool.check_secrets("test.py", content);
        assert!(!findings.is_empty());

        // Should not flag short strings
        let content = r#"name = "hello""#;
        let findings = tool.check_secrets("test.py", content);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_prioritize_findings() {
        let tool = CodeReviewTool::new();

        let findings = vec![
            Finding {
                severity: Severity::Low,
                category: "lint".to_string(),
                message: "Low lint".to_string(),
                file: "test.rs".to_string(),
                line: Some(1),
                column: None,
                suggestion: None,
                analyzer: "clippy".to_string(),
                rule_id: None,
            },
            Finding {
                severity: Severity::Critical,
                category: "security".to_string(),
                message: "Critical security".to_string(),
                file: "test.rs".to_string(),
                line: Some(2),
                column: None,
                suggestion: None,
                analyzer: "semgrep".to_string(),
                rule_id: None,
            },
            Finding {
                severity: Severity::High,
                category: "lint".to_string(),
                message: "High lint".to_string(),
                file: "test.rs".to_string(),
                line: Some(3),
                column: None,
                suggestion: None,
                analyzer: "clippy".to_string(),
                rule_id: None,
            },
        ];

        let prioritized = tool.prioritize_findings(findings);

        // Security should be first
        assert_eq!(prioritized[0].category, "security");
        // Then by severity
        assert_eq!(prioritized[1].severity, Severity::High);
        assert_eq!(prioritized[2].severity, Severity::Low);
    }

    #[tokio::test]
    async fn test_execute_default_params() {
        let tool = CodeReviewTool::new();
        let params = json!({});

        // This should work even if not in a git repo
        let result = tool.execute(params).await;
        assert!(result.is_ok());

        let result = result.unwrap();
        assert!(result.get("findings").is_some());
        assert!(result.get("summary").is_some());
    }
}
