use crate::{CodeSearchError, Result};
use arkavo_mcp::{Tool, ToolSchema};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

pub struct CodeGrepTool {
    schema: ToolSchema,
}

impl CodeGrepTool {
    pub fn new() -> Self {
        Self::validate_dependencies();
        Self {
            schema: ToolSchema {
                name: "codegrep_search".to_string(),
                aliases: None,
                description: "Fast repository-wide code search using ripgrep".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Regular expression pattern to search for"
                        },
                        "path": {
                            "type": "string",
                            "description": "Directory or file to search (defaults to current directory)"
                        },
                        "glob": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Glob patterns to filter files (e.g., ['*.rs', '*.ts'])"
                        },
                        "output_mode": {
                            "type": "string",
                            "enum": ["content", "files", "count"],
                            "description": "Output mode: content (matching lines), files (file paths), count (match counts)"
                        },
                        "case_insensitive": {
                            "type": "boolean",
                            "description": "Case insensitive search"
                        },
                        "context_before": {
                            "type": "integer",
                            "description": "Number of lines to show before match"
                        },
                        "context_after": {
                            "type": "integer",
                            "description": "Number of lines to show after match"
                        },
                        "line_numbers": {
                            "type": "boolean",
                            "description": "Show line numbers (only for content mode)"
                        },
                        "max_results": {
                            "type": "integer",
                            "description": "Maximum number of results to return"
                        }
                    },
                    "required": ["pattern"]
                }),
            },
        }
    }

    fn validate_dependencies() {
        if std::process::Command::new("rg")
            .arg("--version")
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(true)
        {
            tracing::warn!(
                "ripgrep (rg) not found. Install with: brew install ripgrep (macOS) or cargo install ripgrep"
            );
        }
    }

    async fn execute_ripgrep(&self, params: &Value) -> Result<String> {
        let pattern = params["pattern"]
            .as_str()
            .ok_or_else(|| CodeSearchError::InvalidPattern("Missing pattern".to_string()))?;

        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let output_mode = params
            .get("output_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("files");

        let mut cmd = Command::new("rg");

        cmd.arg(pattern).arg(path);

        if params.get("case_insensitive").and_then(|v| v.as_bool()) == Some(true) {
            cmd.arg("-i");
        }

        if params.get("line_numbers").and_then(|v| v.as_bool()) == Some(true) {
            cmd.arg("-n");
        }

        if let Some(before) = params.get("context_before").and_then(|v| v.as_u64()) {
            cmd.arg("-B").arg(before.to_string());
        }

        if let Some(after) = params.get("context_after").and_then(|v| v.as_u64()) {
            cmd.arg("-A").arg(after.to_string());
        }

        if let Some(max) = params.get("max_results").and_then(|v| v.as_u64()) {
            cmd.arg("-m").arg(max.to_string());
        }

        match output_mode {
            "files" => {
                cmd.arg("-l");
            }
            "count" => {
                cmd.arg("-c");
            }
            "content" => {}
            _ => {
                return Err(CodeSearchError::ToolError(format!(
                    "Invalid output_mode: {output_mode}"
                )));
            }
        }

        if let Some(globs) = params.get("glob").and_then(|v| v.as_array()) {
            for glob_pattern in globs {
                if let Some(g) = glob_pattern.as_str() {
                    cmd.arg("-g").arg(g);
                }
            }
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| CodeSearchError::RipgrepError(format!("Failed to spawn ripgrep: {e}")))?;

        let mut stdout = String::new();
        let mut stderr = String::new();

        if let Some(mut stdout_stream) = child.stdout.take() {
            stdout_stream
                .read_to_string(&mut stdout)
                .await
                .map_err(CodeSearchError::IoError)?;
        }

        if let Some(mut stderr_stream) = child.stderr.take() {
            stderr_stream
                .read_to_string(&mut stderr)
                .await
                .map_err(CodeSearchError::IoError)?;
        }

        let status = child
            .wait()
            .await
            .map_err(|e| CodeSearchError::RipgrepError(format!("Wait failed: {e}")))?;

        if !status.success() && status.code() != Some(1) {
            return Err(CodeSearchError::RipgrepError(format!(
                "ripgrep failed: {stderr}"
            )));
        }

        Ok(stdout)
    }
}

impl Default for CodeGrepTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for CodeGrepTool {
    async fn execute(
        &self,
        params: Value,
    ) -> std::result::Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let output = self
            .execute_ripgrep(&params)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let output_mode = params
            .get("output_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("files");

        let results = match output_mode {
            "files" => {
                let files: Vec<String> = output.lines().map(|s| s.to_string()).collect();
                json!({
                    "mode": "files",
                    "count": files.len(),
                    "files": files
                })
            }
            "count" => {
                let counts: Vec<Value> = output
                    .lines()
                    .filter_map(|line| {
                        let parts: Vec<&str> = line.splitn(2, ':').collect();
                        if parts.len() == 2 {
                            Some(json!({
                                "count": parts[0].parse::<u64>().unwrap_or(0),
                                "file": parts[1]
                            }))
                        } else {
                            None
                        }
                    })
                    .collect();
                json!({
                    "mode": "count",
                    "results": counts
                })
            }
            "content" => {
                let lines: Vec<String> = output.lines().map(|s| s.to_string()).collect();
                json!({
                    "mode": "content",
                    "count": lines.len(),
                    "lines": lines
                })
            }
            _ => json!({ "error": "Invalid mode" }),
        };

        Ok(results)
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::fs;

    fn is_ripgrep_available() -> bool {
        std::process::Command::new("rg")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn test_codegrep_files() {
        if !is_ripgrep_available() {
            eprintln!("Skipping test: ripgrep not installed");
            return;
        }

        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.rs");
        fs::write(&test_file, "async fn test() {}\n").await.unwrap();

        let tool = CodeGrepTool::new();
        let params = json!({
            "pattern": "async fn",
            "path": temp_dir.path().to_str().unwrap(),
            "output_mode": "files",
            "glob": ["*.rs"]
        });

        let result = tool.execute(params).await;
        assert!(result.is_ok(), "Expected Ok but got: {result:?}");
        if let Ok(value) = result {
            assert!(value.get("files").is_some());
        }
    }

    #[tokio::test]
    async fn test_codegrep_content() {
        if !is_ripgrep_available() {
            eprintln!("Skipping test: ripgrep not installed");
            return;
        }

        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.rs");
        fs::write(&test_file, "use std::collections::HashMap;\nfn main() {}\n")
            .await
            .unwrap();

        let tool = CodeGrepTool::new();
        let params = json!({
            "pattern": "use ",
            "path": temp_dir.path().to_str().unwrap(),
            "output_mode": "content",
            "line_numbers": true,
            "max_results": 5
        });

        let result = tool.execute(params).await;
        assert!(result.is_ok(), "Expected Ok but got: {result:?}");
        if let Ok(value) = result {
            assert!(value.get("lines").is_some());
        }
    }
}
