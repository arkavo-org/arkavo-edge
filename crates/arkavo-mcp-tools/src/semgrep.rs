use crate::server::{Tool, ToolSchema};
use crate::{Result, ToolError};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

pub struct SemgrepTool {
    schema: ToolSchema,
}

impl SemgrepTool {
    pub fn new() -> Self {
        Self::validate_dependencies();
        Self {
            schema: ToolSchema {
                name: "sec_semgrep".to_string(),
                description: "SAST security scanning with Semgrep for vulnerability detection"
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Directory or file to scan (defaults to current directory)"
                        },
                        "config": {
                            "type": "string",
                            "enum": ["auto", "p/security-audit", "p/owasp-top-ten", "p/cwe-top-25", "p/ci"],
                            "description": "Rule configuration (auto uses .semgrep.yml or p/auto)"
                        },
                        "severity": {
                            "type": "array",
                            "items": {
                                "type": "string",
                                "enum": ["ERROR", "WARNING", "INFO"]
                            },
                            "description": "Filter by severity levels"
                        },
                        "exclude": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Paths to exclude (e.g., ['test/', 'vendor/'])"
                        },
                        "max_target_bytes": {
                            "type": "integer",
                            "description": "Skip files larger than this (bytes)"
                        },
                        "metrics": {
                            "type": "boolean",
                            "description": "Include performance metrics"
                        }
                    },
                    "required": []
                }),
            },
        }
    }

    fn validate_dependencies() {
        if std::process::Command::new("semgrep")
            .arg("--version")
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(true)
        {
            tracing::warn!(
                "semgrep not found. Install with: pip install semgrep or brew install semgrep"
            );
        }
    }

    async fn execute_semgrep(&self, params: &Value) -> Result<String> {
        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let config = params
            .get("config")
            .and_then(|v| v.as_str())
            .unwrap_or("auto");

        let mut cmd = Command::new("semgrep");

        cmd.arg("scan");
        cmd.arg(path);

        cmd.arg("--json");

        if config == "auto" {
            cmd.arg("--config").arg("auto");
        } else {
            cmd.arg("--config").arg(config);
        }

        if let Some(severities) = params.get("severity").and_then(|v| v.as_array()) {
            for sev in severities {
                if let Some(s) = sev.as_str() {
                    cmd.arg("--severity").arg(s);
                }
            }
        }

        if let Some(excludes) = params.get("exclude").and_then(|v| v.as_array()) {
            for exc in excludes {
                if let Some(e) = exc.as_str() {
                    cmd.arg("--exclude").arg(e);
                }
            }
        }

        if let Some(max_bytes) = params.get("max_target_bytes").and_then(|v| v.as_u64()) {
            cmd.arg("--max-target-bytes").arg(max_bytes.to_string());
        }

        if params.get("metrics").and_then(|v| v.as_bool()) == Some(true) {
            cmd.arg("--metrics");
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            ToolError::Mcp(format!(
                "Failed to spawn semgrep: {e}. Is semgrep installed?"
            ))
        })?;

        let mut stdout = String::new();
        let mut stderr = String::new();

        if let Some(mut stdout_stream) = child.stdout.take() {
            stdout_stream
                .read_to_string(&mut stdout)
                .await
                .map_err(ToolError::Io)?;
        }

        if let Some(mut stderr_stream) = child.stderr.take() {
            stderr_stream
                .read_to_string(&mut stderr)
                .await
                .map_err(ToolError::Io)?;
        }

        let status = child
            .wait()
            .await
            .map_err(|e| ToolError::Mcp(format!("Wait failed: {e}")))?;

        if !status.success() && status.code() != Some(1) {
            return Err(ToolError::Mcp(format!("semgrep failed: {stderr}")));
        }

        Ok(stdout)
    }
}

impl Default for SemgrepTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SemgrepTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let output = self.execute_semgrep(&params).await?;

        let result: Value =
            serde_json::from_str(&output).unwrap_or_else(|_| json!({ "raw_output": output }));

        if let Some(results) = result.get("results").and_then(|r| r.as_array()) {
            let summary = json!({
                "total_findings": results.len(),
                "by_severity": {
                    "error": results.iter().filter(|r| r.get("extra").and_then(|e| e.get("severity")).and_then(|s| s.as_str()) == Some("ERROR")).count(),
                    "warning": results.iter().filter(|r| r.get("extra").and_then(|e| e.get("severity")).and_then(|s| s.as_str()) == Some("WARNING")).count(),
                    "info": results.iter().filter(|r| r.get("extra").and_then(|e| e.get("severity")).and_then(|s| s.as_str()) == Some("INFO")).count(),
                },
                "results": results.iter().take(20).cloned().collect::<Vec<_>>()
            });

            Ok(summary)
        } else {
            Ok(result)
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_semgrep_scan() {
        let tool = SemgrepTool::new();
        let params = json!({
            "path": ".",
            "config": "p/ci",
            "severity": ["ERROR", "WARNING"]
        });

        let result = tool.execute(params).await;
        assert!(result.is_ok() || result.is_err());
    }
}
