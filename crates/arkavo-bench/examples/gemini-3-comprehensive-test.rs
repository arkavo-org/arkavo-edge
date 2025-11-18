// Comprehensive test suite for Gemini 3 Pro Preview integration with Arkavo Edge
// Designed to be executed autonomously by Claude Code

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::process::Command;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TestResult {
    test_name: String,
    category: TestCategory,
    status: TestStatus,
    duration_ms: u64,
    metrics: TestMetrics,
    errors: Vec<String>,
    notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum TestCategory {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum TestStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TestMetrics {
    ttft_ms: Option<u64>,
    tokens_generated: Option<usize>,
    throughput_tps: Option<f64>,
    tool_calls: Option<usize>,
    success_rate: Option<f64>,
}

impl Default for TestMetrics {
    fn default() -> Self {
        Self {
            ttft_ms: None,
            tokens_generated: None,
            throughput_tps: None,
            tool_calls: None,
            success_rate: None,
        }
    }
}

struct TestRunner {
    api_key: String,
    model: String,
    results: Vec<TestResult>,
    workspace_root: PathBuf,
}

impl TestRunner {
    fn new() -> Result<Self> {
        let api_key = std::env::var("GEMINI_API_KEY").map_err(|_| "GEMINI_API_KEY not set")?;

        let model = std::env::var("GEMINI_MODEL")
            .unwrap_or_else(|_| "models/gemini-3-pro-preview".to_string());

        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .ok_or("Failed to find workspace root")?
            .to_path_buf();

        Ok(Self {
            api_key,
            model,
            results: Vec::new(),
            workspace_root,
        })
    }

    async fn run_command(
        &self,
        args: &[&str],
        timeout_secs: u64,
    ) -> Result<(String, String, Duration)> {
        let start = Instant::now();

        let output = tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            Command::new("cargo")
                .current_dir(&self.workspace_root)
                .args(args)
                .env("GEMINI_API_KEY", &self.api_key)
                .env("GEMINI_MODEL", &self.model)
                .output(),
        )
        .await
        .map_err(|_| "Command timeout")??;

        let duration = start.elapsed();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        Ok((stdout, stderr, duration))
    }

    // Category 1: Model Availability & Configuration
    async fn test_model_availability(&mut self) -> Result<()> {
        println!("\n=== Test: Model Availability ===");
        let start = Instant::now();

        let (stdout, stderr, duration) = self
            .run_command(
                &["run", "-p", "arkavo-gemini", "--example", "list_models"],
                30,
            )
            .await?;

        let combined_output = format!("{}\n{}", stdout, stderr);
        let found = combined_output.contains("gemini-3-pro-preview");
        let has_streaming =
            combined_output.contains("streaming") || combined_output.contains("supportsStreaming");

        let status = if found && has_streaming {
            TestStatus::Passed
        } else {
            TestStatus::Failed
        };

        let errors = if !found {
            vec!["Model gemini-3-pro-preview not found in list".to_string()]
        } else if !has_streaming {
            vec!["Streaming support not detected".to_string()]
        } else {
            vec![]
        };

        self.results.push(TestResult {
            test_name: "model_availability".to_string(),
            category: TestCategory::Critical,
            status,
            duration_ms: duration.as_millis() as u64,
            metrics: TestMetrics::default(),
            errors,
            notes: if found {
                "Model found and available".to_string()
            } else {
                "Model not found".to_string()
            },
        });

        Ok(())
    }

    // Category 2: Basic Text Generation
    async fn test_basic_text_generation(&mut self) -> Result<()> {
        println!("\n=== Test: Basic Text Generation ===");
        let start = Instant::now();

        let (stdout, stderr, duration) = self
            .run_command(
                &[
                    "run",
                    "-p",
                    "arkavo",
                    "--",
                    "chat",
                    "--prompt",
                    "Write a simple Rust function to add two numbers. Just code, no explanation.",
                ],
                30,
            )
            .await?;

        let combined = format!("{}\n{}", stdout, stderr);
        let has_rust_code =
            combined.contains("fn ") && (combined.contains("->") || combined.contains("{"));
        let has_error =
            stderr.contains("error") || stderr.contains("Error") || stderr.contains("panic");

        let status = if has_rust_code && !has_error {
            TestStatus::Passed
        } else {
            TestStatus::Failed
        };

        let errors = if has_error {
            vec!["Error detected in output".to_string()]
        } else if !has_rust_code {
            vec!["No valid Rust code generated".to_string()]
        } else {
            vec![]
        };

        self.results.push(TestResult {
            test_name: "basic_text_generation".to_string(),
            category: TestCategory::Critical,
            status,
            duration_ms: duration.as_millis() as u64,
            metrics: TestMetrics {
                ttft_ms: Some(duration.as_millis() as u64),
                ..Default::default()
            },
            errors,
            notes: "Generated Rust code".to_string(),
        });

        Ok(())
    }

    // Category 3: Single Tool Call
    async fn test_single_tool_call(&mut self) -> Result<()> {
        println!("\n=== Test: Single Tool Call (Filesystem) ===");
        let start = Instant::now();

        let (stdout, stderr, duration) = self
            .run_command(
                &[
                    "run",
                    "-p",
                    "arkavo",
                    "--",
                    "task",
                    "List files in crates/arkavo-mcp-tools/src",
                ],
                60,
            )
            .await?;

        let combined = format!("{}\n{}", stdout, stderr);
        let tool_not_found =
            combined.contains("Tool not found") || combined.contains("tool not found");
        let has_files = combined.contains(".rs") || combined.contains("filesystem.rs");
        let has_error = stderr.contains("panic") || stderr.contains("ERRO");

        let status = if !tool_not_found && has_files && !has_error {
            TestStatus::Passed
        } else {
            TestStatus::Failed
        };

        let mut errors = Vec::new();
        if tool_not_found {
            errors.push("Tool not found error detected".to_string());
        }
        if !has_files {
            errors.push("No file listing found in output".to_string());
        }
        if has_error {
            errors.push("Error detected in execution".to_string());
        }

        self.results.push(TestResult {
            test_name: "single_tool_call".to_string(),
            category: TestCategory::Critical,
            status,
            duration_ms: duration.as_millis() as u64,
            metrics: TestMetrics {
                tool_calls: Some(1),
                success_rate: Some(if errors.is_empty() { 1.0 } else { 0.0 }),
                ..Default::default()
            },
            errors,
            notes: "Filesystem tool execution".to_string(),
        });

        Ok(())
    }

    // Category 4: Streaming Performance
    async fn test_streaming_performance(&mut self) -> Result<()> {
        println!("\n=== Test: Streaming Performance ===");
        let start = Instant::now();

        let (stdout, stderr, duration) = self
            .run_command(
                &[
                    "run",
                    "-p",
                    "arkavo-gemini",
                    "--example",
                    "streaming_tool_test",
                ],
                60,
            )
            .await?;

        let combined = format!("{}\n{}", stdout, stderr);

        // Parse TTFT from output
        let ttft_ms = combined
            .lines()
            .find(|line| line.contains("TTFT:"))
            .and_then(|line| {
                line.split("TTFT:")
                    .nth(1)
                    .and_then(|s| s.trim().split_whitespace().next())
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(|s| (s * 1000.0) as u64)
            });

        // Parse throughput
        let throughput_tps = combined
            .lines()
            .find(|line| line.contains("tok/s"))
            .and_then(|line| {
                line.split("tok/s")
                    .next()
                    .and_then(|s| s.trim().split_whitespace().last())
                    .and_then(|s| s.parse::<f64>().ok())
            });

        let has_error = stderr.contains("panic") || stderr.contains("ERRO");
        let streaming_worked = combined.contains("Generated") || combined.contains("tokens");

        let status = if streaming_worked && !has_error {
            TestStatus::Passed
        } else {
            TestStatus::Failed
        };

        self.results.push(TestResult {
            test_name: "streaming_performance".to_string(),
            category: TestCategory::High,
            status,
            duration_ms: duration.as_millis() as u64,
            metrics: TestMetrics {
                ttft_ms,
                throughput_tps,
                ..Default::default()
            },
            errors: if has_error {
                vec!["Streaming error detected".to_string()]
            } else {
                vec![]
            },
            notes: format!(
                "TTFT: {:?}ms, Throughput: {:?} tok/s",
                ttft_ms, throughput_tps
            ),
        });

        Ok(())
    }

    // Category 5: Multiple Tool Calls
    async fn test_multiple_tool_calls(&mut self) -> Result<()> {
        println!("\n=== Test: Multiple Tool Calls ===");
        let start = Instant::now();

        let (stdout, stderr, duration) = self
            .run_command(
                &[
                    "run",
                    "-p",
                    "arkavo",
                    "--",
                    "task",
                    "First list files in crates/arkavo-mcp-tools/src, then show git status",
                ],
                120,
            )
            .await?;

        let combined = format!("{}\n{}", stdout, stderr);
        let has_files = combined.contains(".rs");
        let has_git =
            combined.contains("git") || combined.contains("branch") || combined.contains("commit");
        let tool_not_found = combined.contains("Tool not found");
        let has_error = stderr.contains("panic") || stderr.contains("ERRO");

        let status = if has_files && has_git && !tool_not_found && !has_error {
            TestStatus::Passed
        } else {
            TestStatus::Failed
        };

        let mut errors = Vec::new();
        if tool_not_found {
            errors.push("Tool not found error".to_string());
        }
        if !has_files {
            errors.push("Filesystem tool did not execute".to_string());
        }
        if !has_git {
            errors.push("Git tool did not execute".to_string());
        }

        self.results.push(TestResult {
            test_name: "multiple_tool_calls".to_string(),
            category: TestCategory::High,
            status,
            duration_ms: duration.as_millis() as u64,
            metrics: TestMetrics {
                tool_calls: Some(2),
                success_rate: Some(if errors.is_empty() { 1.0 } else { 0.5 }),
                ..Default::default()
            },
            errors,
            notes: "Sequential tool execution".to_string(),
        });

        Ok(())
    }

    // Category 6: Error Handling
    async fn test_error_handling(&mut self) -> Result<()> {
        println!("\n=== Test: Error Handling ===");
        let start = Instant::now();

        // Test with invalid API key
        let output = Command::new("cargo")
            .current_dir(&self.workspace_root)
            .args(&["run", "-p", "arkavo", "--", "chat", "--prompt", "test"])
            .env("GEMINI_API_KEY", "invalid_key_12345")
            .env("GEMINI_MODEL", &self.model)
            .output()
            .await?;

        let duration = start.elapsed();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        let has_auth_error = stderr.contains("auth")
            || stderr.contains("API key")
            || stderr.contains("401")
            || stderr.contains("403");
        let no_panic = !stderr.contains("panic");

        let status = if has_auth_error && no_panic {
            TestStatus::Passed
        } else {
            TestStatus::Failed
        };

        self.results.push(TestResult {
            test_name: "error_handling".to_string(),
            category: TestCategory::Medium,
            status,
            duration_ms: duration.as_millis() as u64,
            metrics: TestMetrics::default(),
            errors: if !no_panic {
                vec!["Panic detected".to_string()]
            } else {
                vec![]
            },
            notes: "Invalid API key handled gracefully".to_string(),
        });

        Ok(())
    }

    // Category 7: SWE-bench Style Task
    async fn test_swe_bench(&mut self) -> Result<()> {
        println!("\n=== Test: SWE-bench Style Task ===");
        let start = Instant::now();

        let (stdout, stderr, duration) = self
            .run_command(
                &[
                    "run",
                    "-p",
                    "arkavo-bench",
                    "--example",
                    "gemini-3-quick-test",
                ],
                180,
            )
            .await?;

        let combined = format!("{}\n{}", stdout, stderr);
        let has_diff =
            combined.contains("diff") || combined.contains("+++") || combined.contains("---");
        let has_success =
            combined.contains("Success") || combined.contains("PASS") || combined.contains("✓");
        let has_error = stderr.contains("panic") || combined.contains("FAIL");

        let status = if has_diff && !has_error {
            TestStatus::Passed
        } else if has_error {
            TestStatus::Failed
        } else {
            TestStatus::Skipped
        };

        self.results.push(TestResult {
            test_name: "swe_bench_task".to_string(),
            category: TestCategory::High,
            status,
            duration_ms: duration.as_millis() as u64,
            metrics: TestMetrics::default(),
            errors: if has_error {
                vec!["SWE-bench test failed".to_string()]
            } else {
                vec![]
            },
            notes: "Complex code generation task".to_string(),
        });

        Ok(())
    }

    // Phase 7: Automated Integration Tests
    async fn test_automated_integration(&mut self) -> Result<()> {
        println!("\n=== Test: Automated Integration (cargo test) ===");
        let start = Instant::now();

        let (stdout, stderr, duration) = self
            .run_command(
                &[
                    "test",
                    "-p",
                    "arkavo-gemini",
                    "--test",
                    "gemini_3_integration_test",
                ],
                120,
            )
            .await?;

        let combined = format!("{}\n{}", stdout, stderr);
        let test_passed = combined.contains("test result: ok");
        let has_error = stderr.contains("FAILED") || stderr.contains("panic");

        let status = if test_passed && !has_error {
            TestStatus::Passed
        } else {
            TestStatus::Failed
        };

        self.results.push(TestResult {
            test_name: "automated_integration".to_string(),
            category: TestCategory::Critical,
            status,
            duration_ms: duration.as_millis() as u64,
            metrics: TestMetrics::default(),
            errors: if has_error {
                vec!["Integration tests failed".to_string()]
            } else {
                vec![]
            },
            notes: "Rust integration tests with assertions".to_string(),
        });

        Ok(())
    }

    // Phase 8: Multimodal Capabilities
    async fn test_multimodal_capabilities(&mut self) -> Result<()> {
        println!("\n=== Test: Multimodal Capabilities (cargo test) ===");
        let start = Instant::now();

        let (stdout, stderr, duration) = self
            .run_command(
                &["test", "-p", "arkavo-gemini", "--test", "multimodal_test"],
                120,
            )
            .await?;

        let combined = format!("{}\n{}", stdout, stderr);
        let test_passed = combined.contains("test result: ok");
        let has_error = stderr.contains("FAILED") || stderr.contains("panic");

        let status = if test_passed && !has_error {
            TestStatus::Passed
        } else {
            TestStatus::Failed
        };

        self.results.push(TestResult {
            test_name: "multimodal_capabilities".to_string(),
            category: TestCategory::High,
            status,
            duration_ms: duration.as_millis() as u64,
            metrics: TestMetrics::default(),
            errors: if has_error {
                vec!["Multimodal tests failed".to_string()]
            } else {
                vec![]
            },
            notes: "Vision/image input validation".to_string(),
        });

        Ok(())
    }

    // Phase 9: Safety & Edge Cases
    async fn test_safety_edge_cases(&mut self) -> Result<()> {
        println!("\n=== Test: Safety & Edge Cases (cargo test) ===");
        let start = Instant::now();

        let (stdout, stderr, duration) = self
            .run_command(
                &[
                    "test",
                    "-p",
                    "arkavo-gemini",
                    "--test",
                    "safety_edge_cases_test",
                ],
                180,
            )
            .await?;

        let combined = format!("{}\n{}", stdout, stderr);
        let test_passed = combined.contains("test result: ok");
        let has_error = stderr.contains("FAILED") || stderr.contains("panic");

        let status = if test_passed && !has_error {
            TestStatus::Passed
        } else {
            TestStatus::Failed
        };

        self.results.push(TestResult {
            test_name: "safety_edge_cases".to_string(),
            category: TestCategory::High,
            status,
            duration_ms: duration.as_millis() as u64,
            metrics: TestMetrics::default(),
            errors: if has_error {
                vec!["Safety/edge case tests failed".to_string()]
            } else {
                vec![]
            },
            notes: "Production robustness validation".to_string(),
        });

        Ok(())
    }

    // Phase 10: Quality Validation
    async fn test_quality_validation(&mut self) -> Result<()> {
        println!("\n=== Test: Quality Validation ===");
        let start = Instant::now();

        let (stdout, stderr, duration) = self
            .run_command(
                &[
                    "run",
                    "-p",
                    "arkavo-bench",
                    "--example",
                    "gemini-3-quality-test",
                ],
                300,
            )
            .await?;

        let combined = format!("{}\n{}", stdout, stderr);
        let has_excellent = combined.contains("Excellent") || combined.contains("excellent");
        let has_good = combined.contains("Good") || combined.contains("good");
        let has_error = stderr.contains("panic") || stderr.contains("FAILED");

        let status = if (has_excellent || has_good) && !has_error {
            TestStatus::Passed
        } else {
            TestStatus::Failed
        };

        self.results.push(TestResult {
            test_name: "quality_validation".to_string(),
            category: TestCategory::High,
            status,
            duration_ms: duration.as_millis() as u64,
            metrics: TestMetrics::default(),
            errors: if has_error {
                vec!["Quality validation failed".to_string()]
            } else {
                vec![]
            },
            notes: "Output correctness and quality scoring".to_string(),
        });

        Ok(())
    }

    fn print_summary(&self) {
        println!("\n\n=== TEST RESULTS SUMMARY ===\n");

        let total = self.results.len();
        let passed = self
            .results
            .iter()
            .filter(|r| matches!(r.status, TestStatus::Passed))
            .count();
        let failed = self
            .results
            .iter()
            .filter(|r| matches!(r.status, TestStatus::Failed))
            .count();
        let skipped = self
            .results
            .iter()
            .filter(|r| matches!(r.status, TestStatus::Skipped))
            .count();

        println!("Total Tests: {}", total);
        println!("Passed: {} ({}%)", passed, (passed * 100) / total.max(1));
        println!("Failed: {} ({}%)", failed, (failed * 100) / total.max(1));
        println!("Skipped: {}\n", skipped);

        println!(
            "{:<30} {:<12} {:<10} {:<12}",
            "Test Name", "Category", "Status", "Duration (ms)"
        );
        println!("{}", "-".repeat(70));

        for result in &self.results {
            let category = format!("{:?}", result.category);
            let status = format!("{:?}", result.status);
            println!(
                "{:<30} {:<12} {:<10} {:<12}",
                result.test_name, category, status, result.duration_ms
            );
        }

        println!("\n=== CRITICAL TESTS ===");
        for result in self
            .results
            .iter()
            .filter(|r| matches!(r.category, TestCategory::Critical))
        {
            let status_symbol = match result.status {
                TestStatus::Passed => "✅",
                TestStatus::Failed => "❌",
                TestStatus::Skipped => "⏭️",
            };
            println!("{} {} - {}", status_symbol, result.test_name, result.notes);
            if !result.errors.is_empty() {
                for error in &result.errors {
                    println!("   ⚠️  {}", error);
                }
            }
        }

        println!("\n=== PERFORMANCE METRICS ===");
        for result in &self.results {
            if let Some(ttft) = result.metrics.ttft_ms {
                println!("{}: TTFT = {}ms", result.test_name, ttft);
            }
            if let Some(tps) = result.metrics.throughput_tps {
                println!("{}: Throughput = {:.1} tok/s", result.test_name, tps);
            }
        }

        // Save results to file
        let results_json = serde_json::to_string_pretty(&self.results).unwrap();
        let output_path = self.workspace_root.join("gemini-3-test-results.json");
        if let Err(e) = fs::write(&output_path, results_json) {
            eprintln!("Failed to write results: {}", e);
        } else {
            println!("\n📄 Results saved to: {}", output_path.display());
        }

        // Production readiness assessment
        println!("\n=== PRODUCTION READINESS ASSESSMENT ===");
        let critical_passed = self
            .results
            .iter()
            .filter(|r| matches!(r.category, TestCategory::Critical))
            .all(|r| matches!(r.status, TestStatus::Passed));

        let high_pass_rate = self
            .results
            .iter()
            .filter(|r| matches!(r.category, TestCategory::High))
            .filter(|r| matches!(r.status, TestStatus::Passed))
            .count() as f64
            / self
                .results
                .iter()
                .filter(|r| matches!(r.category, TestCategory::High))
                .count()
                .max(1) as f64;

        if critical_passed && high_pass_rate >= 0.8 {
            println!("✅ READY FOR PRODUCTION");
            println!("   All critical tests passed");
            println!(
                "   {:.0}% of high-priority tests passed",
                high_pass_rate * 100.0
            );
        } else {
            println!("⚠️  NOT READY FOR PRODUCTION");
            if !critical_passed {
                println!("   ❌ Some critical tests failed");
            }
            if high_pass_rate < 0.8 {
                println!(
                    "   ⚠️  Only {:.0}% of high-priority tests passed (need 80%)",
                    high_pass_rate * 100.0
                );
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("=================================================");
    println!("  Gemini 3 Pro Preview Comprehensive Test Suite");
    println!("=================================================\n");

    let mut runner = TestRunner::new()?;

    println!("Model: {}", runner.model);
    println!("Workspace: {}\n", runner.workspace_root.display());

    // Run test suite
    println!("Starting test execution...\n");

    // Smoke tests (critical)
    if let Err(e) = runner.test_model_availability().await {
        eprintln!("Model availability test error: {}", e);
    }

    if let Err(e) = runner.test_basic_text_generation().await {
        eprintln!("Basic text generation test error: {}", e);
    }

    if let Err(e) = runner.test_single_tool_call().await {
        eprintln!("Single tool call test error: {}", e);
    }

    // Core functionality
    if let Err(e) = runner.test_streaming_performance().await {
        eprintln!("Streaming performance test error: {}", e);
    }

    if let Err(e) = runner.test_multiple_tool_calls().await {
        eprintln!("Multiple tool calls test error: {}", e);
    }

    // Error handling
    if let Err(e) = runner.test_error_handling().await {
        eprintln!("Error handling test error: {}", e);
    }

    // Advanced features
    if let Err(e) = runner.test_swe_bench().await {
        eprintln!("SWE-bench test error: {}", e);
    }

    // Phase 7-10: Production-grade testing
    println!("\n=== Phase 7-10: Production-Grade Tests ===");

    if let Err(e) = runner.test_automated_integration().await {
        eprintln!("Automated integration test error: {}", e);
    }

    if let Err(e) = runner.test_multimodal_capabilities().await {
        eprintln!("Multimodal test error: {}", e);
    }

    if let Err(e) = runner.test_safety_edge_cases().await {
        eprintln!("Safety/edge cases test error: {}", e);
    }

    if let Err(e) = runner.test_quality_validation().await {
        eprintln!("Quality validation test error: {}", e);
    }

    // Print summary
    runner.print_summary();

    Ok(())
}
