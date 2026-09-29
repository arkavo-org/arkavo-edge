use crate::{CodeSearchError, Result};
use arkavo_mcp::{Tool, ToolSchema};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

pub struct CombyTool {
    schema: ToolSchema,
    root: PathBuf,
}

impl CombyTool {
    pub fn new() -> Self {
        Self::with_root(arkavo_validation::current_workspace_root())
    }

    /// Searches only inside `root`; a role that needs a wider root gets it
    /// here, at construction, rather than through a default.
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self::validate_dependencies();
        Self {
            root: root.into(),
            schema: ToolSchema {
                name: "struct_find_replace".to_string(),
                aliases: None,
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
        // comby's parser reads any argument beginning with '-' as a flag, even
        // after `--` (which it rejects as an unknown flag), so a template such
        // as `-review` would change what comby does. Refuse it before spawning.
        if match_template.starts_with('-') || rewrite_template.is_some_and(|r| r.starts_with('-')) {
            return Err(CodeSearchError::InvalidPattern(
                "comby templates cannot begin with '-'".to_string(),
            ));
        }

        let requested = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let path = arkavo_validation::resolve_within_root(&self.root, requested)
            .map_err(|e| CodeSearchError::OutsideWorkspace(e.to_string()))?;
        let in_place = params
            .get("in_place")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut cmd = Command::new("comby");

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

        // Positionals last. Neither template begins with '-' and the confined
        // path is absolute, so none of them can be read as a flag.
        cmd.arg(match_template)
            .arg(rewrite_template.unwrap_or(""))
            .arg(&path);

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
    use arkavo_test_macros::spec;
    use tempfile::TempDir;
    use tokio::fs;

    fn comby_available() -> bool {
        std::process::Command::new("comby")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    /// comby reads any argument beginning with '-' as a flag and rejects `--`
    /// as an unknown flag, so such a template is refused before comby runs.
    #[spec("CS-006")]
    #[tokio::test]
    async fn comby_leading_dash_template_is_refused_not_parsed_as_a_flag() {
        let dir = TempDir::new().unwrap();
        let tool = CombyTool::with_root(dir.path());
        for params in [
            json!({ "match_template": "-> :[x]", "path": dir.path().to_str().unwrap() }),
            json!({
                "match_template": "x",
                "rewrite_template": "-editor=sh",
                "path": dir.path().to_str().unwrap()
            }),
        ] {
            let err = tool.execute(params).await.unwrap_err();
            assert!(
                matches!(
                    err.downcast_ref::<CodeSearchError>(),
                    Some(CodeSearchError::InvalidPattern(_))
                ),
                "{err}"
            );
        }
    }

    /// The reordered argv (flags first, then the templates and the confined
    /// absolute path) is accepted by comby and still finds matches.
    #[spec("CS-006")]
    #[tokio::test]
    async fn comby_searches_a_confined_file() {
        if !comby_available() {
            eprintln!("skip: no comby");
            return;
        }
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.rs"), "fn a() -> u8 { 1 }\n")
            .await
            .unwrap();
        let tool = CombyTool::with_root(dir.path());
        let result = tool
            .execute(json!({
                "match_template": "fn :[name]() -> :[ret] { :[body] }",
                "path": "a.rs"
            }))
            .await
            .expect("comby must accept the argv");
        assert_eq!(result["count"], 1, "{result}");
    }

    #[spec("CS-006")]
    #[tokio::test]
    async fn comby_path_outside_workspace_is_refused() {
        let ws = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let tool = CombyTool::with_root(ws.path());
        let err = tool
            .execute(json!({
                "match_template": "x",
                "path": outside.path().to_str().unwrap()
            }))
            .await
            .unwrap_err();
        assert!(
            matches!(
                err.downcast_ref::<CodeSearchError>(),
                Some(CodeSearchError::OutsideWorkspace(_))
            ),
            "{err}"
        );
    }
}
