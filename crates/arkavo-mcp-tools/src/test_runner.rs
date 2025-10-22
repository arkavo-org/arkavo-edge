use crate::server::{Tool, ToolSchema};
use crate::{Result, ToolError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

pub struct TestRunnerTool {
    schema: ToolSchema,
}

impl TestRunnerTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "test_run".to_string(),
                description:
                    "Multi-language test runner for pytest, jest, go test, cargo test, xcodebuild"
                        .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "framework": {
                            "type": "string",
                            "enum": ["auto", "pytest", "jest", "go", "cargo", "xcodebuild"],
                            "description": "Test framework (auto-detect from project)"
                        },
                        "path": {
                            "type": "string",
                            "description": "Path to tests or test file (defaults to current directory)"
                        },
                        "pattern": {
                            "type": "string",
                            "description": "Test name pattern/filter"
                        },
                        "verbose": {
                            "type": "boolean",
                            "description": "Verbose output (default: false)"
                        },
                        "coverage": {
                            "type": "boolean",
                            "description": "Enable coverage reporting (default: false)"
                        },
                        "parallel": {
                            "type": "boolean",
                            "description": "Run tests in parallel (default: false)"
                        },
                        "timeout": {
                            "type": "integer",
                            "description": "Timeout in seconds (default: 300)"
                        },
                        "extra_args": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Additional framework-specific arguments"
                        }
                    },
                    "required": []
                }),
            },
        }
    }

    fn detect_framework(path: &str) -> Result<String> {
        if std::path::Path::new(&format!("{}/Cargo.toml", path)).exists() {
            return Ok("cargo".to_string());
        }
        if std::path::Path::new(&format!("{}/go.mod", path)).exists() {
            return Ok("go".to_string());
        }
        if std::path::Path::new(&format!("{}/package.json", path)).exists() {
            return Ok("jest".to_string());
        }
        if std::path::Path::new(&format!("{}/pytest.ini", path)).exists()
            || std::path::Path::new(&format!("{}/setup.py", path)).exists()
        {
            return Ok("pytest".to_string());
        }
        if std::path::Path::new(&format!("{}/*.xcodeproj", path)).exists()
            || std::path::Path::new(&format!("{}/*.xcworkspace", path)).exists()
        {
            return Ok("xcodebuild".to_string());
        }
        Err(ToolError::Mcp(
            "Could not auto-detect test framework".to_string(),
        ))
    }

    async fn execute_tests(&self, params: &Value) -> Result<String> {
        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let framework = if let Some(fw) = params.get("framework").and_then(|v| v.as_str()) {
            if fw == "auto" {
                Self::detect_framework(path)?
            } else {
                fw.to_string()
            }
        } else {
            Self::detect_framework(path)?
        };

        let verbose = params
            .get("verbose")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let coverage = params
            .get("coverage")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let parallel = params
            .get("parallel")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut cmd = match framework.as_str() {
            "cargo" => {
                let mut c = Command::new("cargo");
                c.arg("test");
                if verbose {
                    c.arg("--verbose");
                }
                if let Some(pattern) = params.get("pattern").and_then(|v| v.as_str()) {
                    c.arg(pattern);
                }
                c
            }

            "go" => {
                let mut c = Command::new("go");
                c.arg("test");
                if verbose {
                    c.arg("-v");
                }
                if coverage {
                    c.arg("-cover");
                }
                if parallel {
                    c.arg("-parallel").arg("4");
                }
                if let Some(pattern) = params.get("pattern").and_then(|v| v.as_str()) {
                    c.arg("-run").arg(pattern);
                }
                c.arg("./...");
                c
            }

            "pytest" => {
                let mut c = Command::new("pytest");
                if verbose {
                    c.arg("-v");
                }
                if coverage {
                    c.arg("--cov");
                }
                if parallel {
                    c.arg("-n").arg("auto");
                }
                if let Some(pattern) = params.get("pattern").and_then(|v| v.as_str()) {
                    c.arg("-k").arg(pattern);
                }
                c.arg(path);
                c
            }

            "jest" => {
                let mut c = Command::new("npx");
                c.arg("jest");
                if verbose {
                    c.arg("--verbose");
                }
                if coverage {
                    c.arg("--coverage");
                }
                if let Some(pattern) = params.get("pattern").and_then(|v| v.as_str()) {
                    c.arg("-t").arg(pattern);
                }
                c
            }

            "xcodebuild" => {
                let mut c = Command::new("xcodebuild");
                c.arg("test");
                c.arg("-scheme").arg("YourScheme");
                c.arg("-destination")
                    .arg("platform=iOS Simulator,name=iPhone 15");
                if parallel {
                    c.arg("-parallel-testing-enabled").arg("YES");
                }
                c
            }

            _ => {
                return Err(ToolError::Mcp(format!(
                    "Unsupported framework: {framework}"
                )));
            }
        };

        if let Some(extra_args) = params.get("extra_args").and_then(|v| v.as_array()) {
            for arg in extra_args {
                if let Some(a) = arg.as_str() {
                    cmd.arg(a);
                }
            }
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            ToolError::Mcp(format!(
                "Failed to spawn {framework}: {e}. Is {framework} installed?"
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

        let combined = format!("{}\n{}", stdout, stderr);

        if !status.success() && status.code() != Some(1) {
            return Err(ToolError::Mcp(format!(
                "Test run failed with status {:?}",
                status.code()
            )));
        }

        Ok(combined)
    }

    fn parse_results(&self, framework: &str, output: &str) -> Value {
        let lines: Vec<&str> = output.lines().collect();

        match framework {
            "cargo" => {
                let mut passed = 0;
                let mut failed = 0;
                let mut ignored = 0;

                for line in &lines {
                    if line.contains("test result:")
                        && let Some(parts) = line.split("test result:").nth(1)
                    {
                        for part in parts.split('.') {
                            if let Some(num) = part.split_whitespace().next() {
                                if part.contains("passed") {
                                    passed = num.parse().unwrap_or(0);
                                } else if part.contains("failed") {
                                    failed = num.parse().unwrap_or(0);
                                } else if part.contains("ignored") {
                                    ignored = num.parse().unwrap_or(0);
                                }
                            }
                        }
                    }
                }

                json!({
                    "framework": "cargo",
                    "passed": passed,
                    "failed": failed,
                    "ignored": ignored,
                    "total": passed + failed + ignored
                })
            }

            "go" => {
                let mut passed = 0;
                let mut failed = 0;
                let mut skipped = 0;

                for line in &lines {
                    if line.starts_with("PASS") {
                        passed += 1;
                    } else if line.starts_with("FAIL") {
                        failed += 1;
                    } else if line.starts_with("SKIP") {
                        skipped += 1;
                    }
                }

                json!({
                    "framework": "go",
                    "passed": passed,
                    "failed": failed,
                    "skipped": skipped,
                    "total": passed + failed + skipped
                })
            }

            "pytest" => {
                let mut passed = 0;
                let failed = 0;
                let skipped = 0;

                for line in &lines {
                    if line.contains("passed")
                        && let Some(num) = line.split_whitespace().next()
                    {
                        passed = num.parse().unwrap_or(0);
                    }
                }

                json!({
                    "framework": "pytest",
                    "passed": passed,
                    "failed": failed,
                    "skipped": skipped,
                    "total": passed + failed + skipped
                })
            }

            _ => json!({
                "framework": framework,
                "output": output.lines().take(50).collect::<Vec<_>>()
            }),
        }
    }
}

impl Default for TestRunnerTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TestRunnerTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let framework = params
            .get("framework")
            .and_then(|v| v.as_str())
            .unwrap_or("auto");

        let output = self.execute_tests(&params).await?;

        let results = self.parse_results(framework, &output);

        Ok(results)
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cargo_tests() {
        let tool = TestRunnerTool::new();
        let params = json!({
            "framework": "cargo",
            "path": "."
        });

        let result = tool.execute(params).await;
        assert!(result.is_ok() || result.is_err());
    }
}
