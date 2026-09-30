use crate::{CodeSearchError, Result};
use arkavo_mcp::{Tool, ToolSchema};
use arkavo_process_env::ChildEnv;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

fn rg_command() -> std::process::Command {
    // The operator's environment minus credentials: rg needs none, and
    // whatever it runs must not inherit the agent's keys. The tool profile
    // also withholds RIPGREP_CONFIG_PATH, so the arguments built here are
    // the only flags rg sees.
    ChildEnv::tool_from_current(&[]).command("rg")
}

pub struct CodeGrepTool {
    schema: ToolSchema,
    root: PathBuf,
}

impl CodeGrepTool {
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
        if rg_command()
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

        let requested = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let path = arkavo_validation::resolve_within_root(&self.root, requested)
            .map_err(|e| CodeSearchError::OutsideWorkspace(e.to_string()))?;
        let output_mode = params
            .get("output_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("files");

        let mut cmd = Command::from(rg_command());

        // The pattern travels as --regexp=VALUE so a leading '-' can never be
        // read as a flag such as --pre=<command>, which runs a program for
        // every file searched.
        cmd.arg(format!("--regexp={pattern}"));

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

        // The confined path is absolute, but `--` keeps the argv shape safe
        // even if a future caller passes something else.
        cmd.arg("--").arg(&path);

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
    use arkavo_test_macros::spec;
    use std::ffi::OsString;
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

        let tool = CodeGrepTool::with_root(temp_dir.path());
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

        let tool = CodeGrepTool::with_root(temp_dir.path());
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

    /// Ripgrep reads extra flags, `--pre` among them, from the file
    /// `RIPGREP_CONFIG_PATH` names. The control run shows it does; the rg
    /// codegrep starts must see only the flags codegrep gave it.
    #[spec("MCP-016")]
    #[test]
    fn rg_reads_no_flag_file_from_the_environment() {
        if !is_ripgrep_available() {
            eprintln!("skip: no rg");
            return;
        }
        let dir = TempDir::new().unwrap();
        let flag_file = dir.path().join("rgrc");
        std::fs::write(&flag_file, "--replace=HIJACKED\n").unwrap();
        let searched = dir.path().join("f.txt");
        std::fs::write(&searched, "needle\n").unwrap();
        // Neither variable may come from the developer's own environment:
        // an inherited grant would readmit the flag file.
        let mut parent: Vec<(OsString, OsString)> = std::env::vars_os()
            .filter(|(name, _)| {
                name != "RIPGREP_CONFIG_PATH" && name != arkavo_process_env::TOOL_ENV_PASSTHROUGH
            })
            .collect();
        parent.push((
            OsString::from("RIPGREP_CONFIG_PATH"),
            flag_file.into_os_string(),
        ));
        let search = |env: ChildEnv| {
            let output = env
                .command("rg")
                .arg("--regexp=needle")
                .arg("--")
                .arg(&searched)
                .output()
                .expect("spawn rg");
            String::from_utf8_lossy(&output.stdout).into_owned()
        };

        let hijacked = search(ChildEnv::toolchain(parent.clone(), &[]));
        assert!(hijacked.contains("HIJACKED"), "control: {hijacked}");
        let seen = search(ChildEnv::tool(parent, &[], &[]));
        assert_eq!(seen.trim_end(), "needle");
    }

    #[spec("CS-006")]
    #[tokio::test]
    async fn leading_dash_pattern_is_treated_as_a_literal_not_a_flag() {
        if !is_ripgrep_available() {
            eprintln!("skip: no rg");
            return;
        }
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("f.txt"), "value = --pre=oops\n")
            .await
            .unwrap();
        let tool = CodeGrepTool::with_root(dir.path());
        let result = tool
            .execute(json!({
                "pattern": "--pre=oops",
                "path": dir.path().to_str().unwrap(),
                "output_mode": "files"
            }))
            .await;
        // Must succeed and find the file, proving the pattern was a search
        // term and not a consumed --pre flag.
        let v = result.expect("pattern beginning with '-' must be searched literally");
        assert_eq!(v["count"], 1);
    }

    #[spec("CS-006")]
    #[tokio::test]
    async fn codegrep_path_outside_workspace_is_refused() {
        let ws = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let tool = CodeGrepTool::with_root(ws.path());
        let err = tool
            .execute(json!({
                "pattern": "x",
                "path": outside.path().to_str().unwrap(),
                "output_mode": "files"
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
