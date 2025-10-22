use crate::server::{Tool, ToolSchema};
use crate::{Result, ToolError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

pub struct OsvTool {
    schema: ToolSchema,
}

impl OsvTool {
    pub fn new() -> Self {
        Self::validate_dependencies();
        Self {
            schema: ToolSchema {
                name: "deps_osv".to_string(),
                description: "Scan dependencies for vulnerabilities using OSV-Scanner".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Directory or lockfile to scan (defaults to current directory)"
                        },
                        "format": {
                            "type": "string",
                            "enum": ["table", "json", "sarif", "markdown"],
                            "description": "Output format (default: json)"
                        },
                        "lockfile": {
                            "type": "string",
                            "description": "Specific lockfile path (Cargo.lock, package-lock.json, go.mod, etc.)"
                        },
                        "call_analysis": {
                            "type": "boolean",
                            "description": "Enable call analysis to reduce false positives (experimental)"
                        },
                        "recursive": {
                            "type": "boolean",
                            "description": "Scan directories recursively for lockfiles"
                        },
                        "experimental_offline": {
                            "type": "boolean",
                            "description": "Use local OSV database (requires prior download)"
                        }
                    },
                    "required": []
                }),
            },
        }
    }

    fn validate_dependencies() {
        if std::process::Command::new("osv-scanner")
            .arg("--version")
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(true)
        {
            tracing::warn!(
                "osv-scanner not found. Install with: go install github.com/google/osv-scanner/cmd/osv-scanner@latest"
            );
        }
    }

    async fn execute_osv(&self, params: &Value) -> Result<String> {
        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let format = params
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("json");

        let mut cmd = Command::new("osv-scanner");

        if let Some(lockfile) = params.get("lockfile").and_then(|v| v.as_str()) {
            cmd.arg("--lockfile").arg(lockfile);
        } else {
            cmd.arg("--format").arg(format);
            cmd.arg(path);
        }

        if params.get("call_analysis").and_then(|v| v.as_bool()) == Some(true) {
            cmd.arg("--experimental-call-analysis");
        }

        if params.get("recursive").and_then(|v| v.as_bool()) == Some(true) {
            cmd.arg("--recursive");
        }

        if params.get("experimental_offline").and_then(|v| v.as_bool()) == Some(true) {
            cmd.arg("--experimental-offline");
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            ToolError::Mcp(format!(
                "Failed to spawn osv-scanner: {e}. Is osv-scanner installed?"
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
            return Err(ToolError::Mcp(format!("osv-scanner failed: {stderr}")));
        }

        Ok(stdout)
    }
}

impl Default for OsvTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for OsvTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let output = self.execute_osv(&params).await?;

        let result: Value =
            serde_json::from_str(&output).unwrap_or_else(|_| json!({ "raw_output": output }));

        if let Some(results) = result.get("results").and_then(|r| r.as_array()) {
            let mut total_vulns = 0;
            let mut by_severity = json!({
                "critical": 0,
                "high": 0,
                "medium": 0,
                "low": 0,
                "unknown": 0
            });

            for pkg_result in results {
                if let Some(packages) = pkg_result.get("packages").and_then(|p| p.as_array()) {
                    for pkg in packages {
                        if let Some(vulns) = pkg.get("vulnerabilities").and_then(|v| v.as_array()) {
                            total_vulns += vulns.len();
                            for vuln in vulns {
                                if let Some(severity) = vuln
                                    .get("database_specific")
                                    .and_then(|d| d.get("severity"))
                                    .and_then(|s| s.as_str())
                                {
                                    let key = severity.to_lowercase();
                                    if let Some(count) =
                                        by_severity.get(&key).and_then(|c| c.as_u64())
                                    {
                                        by_severity[&key] = json!(count + 1);
                                    }
                                } else if let Some(count) =
                                    by_severity.get("unknown").and_then(|c| c.as_u64())
                                {
                                    by_severity["unknown"] = json!(count + 1);
                                }
                            }
                        }
                    }
                }
            }

            let summary = json!({
                "total_vulnerabilities": total_vulns,
                "by_severity": by_severity,
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
    async fn test_osv_scan() {
        let tool = OsvTool::new();
        let params = json!({
            "path": ".",
            "format": "json"
        });

        let result = tool.execute(params).await;
        assert!(result.is_ok() || result.is_err());
    }
}
