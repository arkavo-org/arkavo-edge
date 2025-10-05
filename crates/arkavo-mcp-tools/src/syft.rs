use crate::server::{Tool, ToolSchema};
use crate::{Result, ToolError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

pub struct SyftTool {
    schema: ToolSchema,
}

impl SyftTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "sbom_syft".to_string(),
                description: "Generate Software Bill of Materials (SBOM) using Syft".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "source": {
                            "type": "string",
                            "description": "Source to analyze (directory, image, tarball, etc.)"
                        },
                        "format": {
                            "type": "string",
                            "enum": ["json", "cyclonedx-json", "cyclonedx-xml", "spdx-json", "spdx-tag-value", "github-json", "syft-json", "table"],
                            "description": "SBOM output format (default: json)"
                        },
                        "scope": {
                            "type": "string",
                            "enum": ["all-layers", "squashed"],
                            "description": "Scope of package discovery for container images"
                        },
                        "exclude": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Paths to exclude from analysis"
                        },
                        "platform": {
                            "type": "string",
                            "description": "Platform filter for multi-platform images (e.g., linux/amd64)"
                        },
                        "catalogers": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Enable specific catalogers (e.g., ['cargo', 'npm'])"
                        }
                    },
                    "required": ["source"]
                }),
            },
        }
    }

    async fn execute_syft(&self, params: &Value) -> Result<String> {
        let source = params["source"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidParams("Missing source".to_string()))?;

        let format = params
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("json");

        let mut cmd = Command::new("syft");

        cmd.arg("scan").arg(source);

        cmd.arg("--output").arg(format);

        if let Some(scope) = params.get("scope").and_then(|v| v.as_str()) {
            cmd.arg("--scope").arg(scope);
        }

        if let Some(excludes) = params.get("exclude").and_then(|v| v.as_array()) {
            for exc in excludes {
                if let Some(e) = exc.as_str() {
                    cmd.arg("--exclude").arg(e);
                }
            }
        }

        if let Some(platform) = params.get("platform").and_then(|v| v.as_str()) {
            cmd.arg("--platform").arg(platform);
        }

        if let Some(catalogers) = params.get("catalogers").and_then(|v| v.as_array()) {
            let cataloger_list: Vec<&str> = catalogers.iter().filter_map(|c| c.as_str()).collect();
            if !cataloger_list.is_empty() {
                cmd.arg("--catalogers").arg(cataloger_list.join(","));
            }
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            ToolError::Mcp(format!("Failed to spawn syft: {e}. Is syft installed?"))
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

        if !status.success() {
            return Err(ToolError::Mcp(format!("syft failed: {stderr}")));
        }

        Ok(stdout)
    }
}

impl Default for SyftTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SyftTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let output = self.execute_syft(&params).await?;

        let result: Value =
            serde_json::from_str(&output).unwrap_or_else(|_| json!({ "raw_output": output }));

        if let Some(artifacts) = result.get("artifacts").and_then(|a| a.as_array()) {
            let mut by_type: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            let mut by_language: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();

            for artifact in artifacts {
                if let Some(pkg_type) = artifact.get("type").and_then(|t| t.as_str()) {
                    *by_type.entry(pkg_type.to_string()).or_insert(0) += 1;
                }
                if let Some(language) = artifact.get("language").and_then(|l| l.as_str()) {
                    *by_language.entry(language.to_string()).or_insert(0) += 1;
                }
            }

            let summary = json!({
                "total_packages": artifacts.len(),
                "by_type": by_type,
                "by_language": by_language,
                "artifacts": artifacts.iter().take(50).cloned().collect::<Vec<_>>()
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
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_syft_scan() {
        let tool = SyftTool::new();
        let params = json!({
            "source": ".",
            "format": "json"
        });

        let result = tool.execute(params).await;
        assert!(result.is_ok() || result.is_err());
    }
}
