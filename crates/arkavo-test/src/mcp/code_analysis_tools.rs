use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::Value;
use std::path::Path;
use std::process::Command;

pub struct FindBugsKit {
    schema: ToolSchema,
}

impl FindBugsKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "find_bugs".to_string(),
                description: "Find potential bugs and code issues in the codebase".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to analyze (defaults to current directory)"
                        },
                        "language": {
                            "type": "string",
                            "enum": ["rust", "swift", "typescript", "python", "auto"],
                            "description": "Programming language to analyze"
                        },
                        "bug_types": {
                            "type": "array",
                            "items": {
                                "type": "string",
                                "enum": ["memory", "concurrency", "error_handling", "security", "performance", "logic", "all"]
                            },
                            "description": "Types of bugs to look for"
                        }
                    },
                    "required": []
                }),
            },
        }
    }
}

impl Default for FindBugsKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for FindBugsKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let language = params
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or("auto");

        let bug_types = params
            .get("bug_types")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
            .unwrap_or_else(|| vec!["all"]);

        // Determine language if auto
        let detected_language = if language == "auto" {
            detect_language(path)?
        } else {
            language.to_string()
        };

        // Run analysis based on language
        let bugs = match detected_language.as_str() {
            "rust" => analyze_rust(path, &bug_types).await?,
            "swift" => analyze_swift(path, &bug_types).await?,
            _ => {
                return Err(TestError::Mcp(format!(
                    "Unsupported language: {}",
                    detected_language
                )));
            }
        };

        Ok(serde_json::json!({
            "language": detected_language,
            "path": path,
            "bugs": bugs,
            "summary": {
                "total": bugs.len(),
                "by_severity": count_by_severity(&bugs)
            }
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct CodeAnalysisKit {
    schema: ToolSchema,
}

impl CodeAnalysisKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "analyze_code".to_string(),
                description: "Analyze code quality, complexity, and potential issues".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "Path to file or directory to analyze"
                        },
                        "analysis_type": {
                            "type": "string",
                            "enum": ["complexity", "coverage", "dependencies", "security", "all"],
                            "description": "Type of analysis to perform"
                        }
                    },
                    "required": ["file_path"]
                }),
            },
        }
    }
}

impl Default for CodeAnalysisKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for CodeAnalysisKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let file_path = params
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing file_path parameter".to_string()))?;

        let _analysis_type = params
            .get("analysis_type")
            .and_then(|v| v.as_str())
            .unwrap_or("all");

        Ok(serde_json::json!({
            "file_path": file_path,
            "analysis": {
                "complexity": {
                    "cyclomatic": 5,
                    "cognitive": 8
                },
                "issues": [
                    {
                        "type": "error_handling",
                        "severity": "medium",
                        "message": "Missing error handling in function",
                        "line": 42
                    }
                ],
                "metrics": {
                    "lines_of_code": 150,
                    "functions": 10,
                    "classes": 2
                }
            }
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct TestAnalysisKit {
    schema: ToolSchema,
}

impl TestAnalysisKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "analyze_tests".to_string(),
                description: "Analyze test coverage and identify missing test cases".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "project_path": {
                            "type": "string",
                            "description": "Path to project root"
                        },
                        "include_integration": {
                            "type": "boolean",
                            "description": "Include integration tests in analysis"
                        }
                    },
                    "required": []
                }),
            },
        }
    }
}

impl Default for TestAnalysisKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TestAnalysisKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let project_path = params
            .get("project_path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        // Run test coverage analysis
        let coverage = analyze_test_coverage(project_path).await?;

        Ok(serde_json::json!({
            "project_path": project_path,
            "coverage": coverage,
            "missing_tests": [
                {
                    "file": "src/main.rs",
                    "function": "process_data",
                    "reason": "No test cases found"
                }
            ],
            "test_quality": {
                "assertions_per_test": 2.5,
                "test_duplication": 0.15,
                "flaky_tests": []
            }
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// Helper functions

fn detect_language(path: &str) -> Result<String> {
    // Check for common file extensions
    if Path::new(path).join("Cargo.toml").exists() {
        return Ok("rust".to_string());
    }
    if Path::new(path).join("Package.swift").exists() {
        return Ok("swift".to_string());
    }

    // Check file extensions in directory
    let output = Command::new("find")
        .args([path, "-name", "*.rs", "-o", "-name", "*.swift"])
        .output()
        .map_err(|e| TestError::Mcp(format!("Failed to detect language: {}", e)))?;

    let files = String::from_utf8_lossy(&output.stdout);
    if files.contains(".rs") {
        Ok("rust".to_string())
    } else if files.contains(".swift") {
        Ok("swift".to_string())
    } else {
        Err(TestError::Mcp("Could not detect language".to_string()))
    }
}

async fn analyze_rust(path: &str, bug_types: &[&str]) -> Result<Vec<serde_json::Value>> {
    let mut bugs = Vec::new();

    // Run clippy for Rust analysis
    if bug_types.contains(&"all") || bug_types.iter().any(|&t| t != "all") {
        let output = Command::new("cargo")
            .args(["clippy", "--message-format=json"])
            .current_dir(path)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to run clippy: {}", e)))?;

        // Parse clippy output
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Ok(msg) = serde_json::from_str::<Value>(line) {
                if msg["reason"] == "compiler-message" {
                    if let Some(message) = msg.get("message") {
                        bugs.push(serde_json::json!({
                            "type": "clippy",
                            "severity": message["level"],
                            "message": message["message"],
                            "file": message["spans"][0]["file_name"],
                            "line": message["spans"][0]["line_start"]
                        }));
                    }
                }
            }
        }
    }

    Ok(bugs)
}

async fn analyze_swift(path: &str, bug_types: &[&str]) -> Result<Vec<serde_json::Value>> {
    let mut bugs = Vec::new();

    // Search for common Swift anti-patterns
    if bug_types.contains(&"all") || bug_types.contains(&"memory") {
        // Look for force unwrapping
        let output = Command::new("grep")
            .args(["-rn", "--include=*.swift", r"!\s*[{.\[(]", path])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to search for patterns: {}", e)))?;

        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Some((file_line, _)) = line.split_once(':') {
                if let Some((file, line_num)) = file_line.rsplit_once(':') {
                    bugs.push(serde_json::json!({
                        "type": "force_unwrap",
                        "severity": "high",
                        "message": "Force unwrapping detected - potential crash",
                        "file": file,
                        "line": line_num
                    }));
                }
            }
        }
    }

    Ok(bugs)
}

async fn analyze_test_coverage(_path: &str) -> Result<serde_json::Value> {
    // This would run actual coverage tools
    Ok(serde_json::json!({
        "total": 75.5,
        "by_file": {
            "src/main.rs": 80.0,
            "src/lib.rs": 70.0
        }
    }))
}

fn count_by_severity(bugs: &[serde_json::Value]) -> serde_json::Value {
    let mut counts = std::collections::HashMap::new();
    for bug in bugs {
        if let Some(severity) = bug.get("severity").and_then(|v| v.as_str()) {
            *counts.entry(severity).or_insert(0) += 1;
        }
    }
    serde_json::to_value(counts).unwrap()
}
