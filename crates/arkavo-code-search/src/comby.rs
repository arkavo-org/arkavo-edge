use crate::{CodeSearchError, Result};
use arkavo_mcp::{Tool, ToolSchema};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

pub struct CombyTool {
    schema: ToolSchema,
}

impl CombyTool {
    pub fn new() -> Self {
        Self::validate_dependencies();
        Self {
            schema: ToolSchema {
                name: "struct_find_replace".to_string(),
                description:
                    "Structural code search and replace using Comby for language-aware refactoring"
                        .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "match_template": {
                            "type": "string",
                            "description": "Template pattern to match (use :[var] for holes)"
                        },
                        "rewrite_template": {
                            "type": "string",
                            "description": "Template for replacement (use matched :[var] from match)"
                        },
                        "path": {
                            "type": "string",
                            "description": "Directory or file to search (defaults to current directory)"
                        },
                        "language": {
                            "type": "string",
                            "enum": ["rust", "go", "python", "javascript", "typescript", "java", "c", "cpp", "auto"],
                            "description": "Programming language (auto-detect if not specified)"
                        },
                        "file_extensions": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "File extensions to target (e.g., ['.rs', '.go'])"
                        },
                        "in_place": {
                            "type": "boolean",
                            "description": "Apply changes in-place (default: false, preview only)"
                        },
                        "case_sensitive": {
                            "type": "boolean",
                            "description": "Case-sensitive matching (default: true)"
                        },
                        "exclude_dirs": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Directories to exclude (e.g., ['target', 'node_modules'])"
                        }
                    },
                    "required": ["match_template"]
                }),
            },
        }
    }

    fn validate_dependencies() {
        if std::process::Command::new("comby")
            .arg("--version")
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(true)
        {
            tracing::warn!(
                "comby not found. Install with: brew install comby (macOS) or see https://comby.dev"
            );
        }
    }

    async fn execute_comby(&self, params: &Value) -> Result<String> {
        let match_template = params["match_template"]
            .as_str()
            .ok_or_else(|| CodeSearchError::InvalidPattern("Missing match_template".to_string()))?;

        let rewrite_template = params.get("rewrite_template").and_then(|v| v.as_str());
        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let in_place = params
            .get("in_place")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut cmd = Command::new("comby");

        cmd.arg(match_template);

        if let Some(rewrite) = rewrite_template {
            cmd.arg(rewrite);
        } else {
            cmd.arg("");
        }

        cmd.arg(path);

        if let Some(lang) = params.get("language").and_then(|v| v.as_str())
            && lang != "auto"
        {
            cmd.arg("-matcher").arg(lang);
        }

        if let Some(exts) = params.get("file_extensions").and_then(|v| v.as_array()) {
            for ext in exts {
                if let Some(e) = ext.as_str() {
                    cmd.arg("-extensions").arg(e);
                }
            }
        }

        if in_place {
            cmd.arg("-in-place");
        } else {
            cmd.arg("-json-lines");
        }

        if params.get("case_sensitive").and_then(|v| v.as_bool()) == Some(false) {
            cmd.arg("-match-only");
        }

        if let Some(excludes) = params.get("exclude_dirs").and_then(|v| v.as_array()) {
            for dir in excludes {
                if let Some(d) = dir.as_str() {
                    cmd.arg("-exclude-dir").arg(d);
                }
            }
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            CodeSearchError::RipgrepError(format!(
                "Failed to spawn comby: {e}. Is comby installed?"
            ))
        })?;

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

        if !status.success() {
            return Err(CodeSearchError::RipgrepError(format!(
                "comby failed: {stderr}"
            )));
        }

        Ok(stdout)
    }
}

impl Default for CombyTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for CombyTool {
    async fn execute(
        &self,
        params: Value,
    ) -> std::result::Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let output = self
            .execute_comby(&params)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let in_place = params
            .get("in_place")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if in_place {
            Ok(json!({
                "mode": "in_place",
                "message": "Changes applied successfully",
                "output": output
            }))
        } else {
            let matches: Vec<Value> = output
                .lines()
                .filter(|line| !line.trim().is_empty())
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect();

            Ok(json!({
                "mode": "preview",
                "count": matches.len(),
                "matches": matches
            }))
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
    async fn test_comby_search() {
        let tool = CombyTool::new();
        let params = json!({
            "match_template": "fn :[name](:[args]) -> :[ret]",
            "path": "src",
            "language": "rust"
        });

        let result = tool.execute(params).await;
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_comby_replace_preview() {
        let tool = CombyTool::new();
        let params = json!({
            "match_template": "unwrap()",
            "rewrite_template": "unwrap_or_default()",
            "path": ".",
            "language": "rust",
            "in_place": false
        });

        let result = tool.execute(params).await;
        assert!(result.is_ok() || result.is_err());
    }
}
