use crate::cognitive_engine_core::{PlanStep, VerificationCheck, VerificationResult};
use crate::error::Result;
use tracing::debug;

pub struct Verifier;

impl Verifier {
    pub fn new() -> Self {
        Self
    }

    pub async fn check(&self, step: &PlanStep) -> Result<Vec<VerificationResult>> {
        debug!(step = step.step_number, "Running verification checks");

        let mut results = Vec::new();

        for check in &step.verification {
            let result = match check {
                VerificationCheck::TestsPassing => self.verify_tests().await,
                VerificationCheck::LinterClean => self.verify_linter().await,
                VerificationCheck::BuildSuccessful => self.verify_build().await,
                VerificationCheck::FileConstraint { max_lines } => {
                    self.verify_file_constraints(*max_lines).await
                }
            };

            results.push(result);
        }

        Ok(results)
    }

    async fn verify_tests(&self) -> VerificationResult {
        debug!("Running tests");

        let output = tokio::process::Command::new("cargo")
            .arg("test")
            .arg("--all")
            .arg("--")
            .arg("--nocapture")
            .output()
            .await;

        match output {
            Ok(result) => {
                let passed = result.status.success();
                let stderr = String::from_utf8_lossy(&result.stderr);
                let stdout = String::from_utf8_lossy(&result.stdout);

                let details = if passed {
                    "All tests passed".to_string()
                } else {
                    format!(
                        "Tests failed:\n{}{}",
                        stdout.lines().take(10).collect::<Vec<_>>().join("\n"),
                        stderr.lines().take(10).collect::<Vec<_>>().join("\n")
                    )
                };

                VerificationResult {
                    check: VerificationCheck::TestsPassing,
                    passed,
                    details,
                }
            }
            Err(e) => VerificationResult {
                check: VerificationCheck::TestsPassing,
                passed: false,
                details: format!("Failed to run tests: {e}"),
            },
        }
    }

    async fn verify_linter(&self) -> VerificationResult {
        debug!("Running linter");

        let output = tokio::process::Command::new("cargo")
            .arg("clippy")
            .arg("--all-targets")
            .arg("--")
            .arg("-D")
            .arg("warnings")
            .output()
            .await;

        match output {
            Ok(result) => {
                let passed = result.status.success();
                let stderr = String::from_utf8_lossy(&result.stderr);
                let stdout = String::from_utf8_lossy(&result.stdout);

                let details = if passed {
                    "Linter checks passed".to_string()
                } else {
                    format!(
                        "Linter warnings/errors:\n{}{}",
                        stdout.lines().take(10).collect::<Vec<_>>().join("\n"),
                        stderr.lines().take(10).collect::<Vec<_>>().join("\n")
                    )
                };

                VerificationResult {
                    check: VerificationCheck::LinterClean,
                    passed,
                    details,
                }
            }
            Err(e) => VerificationResult {
                check: VerificationCheck::LinterClean,
                passed: false,
                details: format!("Failed to run linter: {e}"),
            },
        }
    }

    async fn verify_build(&self) -> VerificationResult {
        debug!("Running build");

        let output = tokio::process::Command::new("cargo")
            .arg("build")
            .arg("--all")
            .output()
            .await;

        match output {
            Ok(result) => {
                let passed = result.status.success();
                let stderr = String::from_utf8_lossy(&result.stderr);
                let stdout = String::from_utf8_lossy(&result.stdout);

                let details = if passed {
                    "Build successful".to_string()
                } else {
                    format!(
                        "Build failed:\n{}{}",
                        stdout.lines().take(10).collect::<Vec<_>>().join("\n"),
                        stderr.lines().take(10).collect::<Vec<_>>().join("\n")
                    )
                };

                VerificationResult {
                    check: VerificationCheck::BuildSuccessful,
                    passed,
                    details,
                }
            }
            Err(e) => VerificationResult {
                check: VerificationCheck::BuildSuccessful,
                passed: false,
                details: format!("Failed to run build: {e}"),
            },
        }
    }

    async fn verify_file_constraints(&self, max_lines: usize) -> VerificationResult {
        debug!(max_lines, "Checking file size constraints");

        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(format!(
                "find . -name '*.rs' -type f ! -path '*/target/*' ! -path '*/vendor/*' -exec wc -l {{}} \\; | awk '$1 > {max_lines} {{print}}'"
            ))
            .output()
            .await;

        match output {
            Ok(result) => {
                let stdout = String::from_utf8_lossy(&result.stdout);
                let violations: Vec<&str> = stdout.lines().collect();

                let passed = violations.is_empty();
                let details = if passed {
                    format!("All Rust files under {max_lines} lines")
                } else {
                    format!(
                        "{} files exceed {max_lines} lines:\n{}",
                        violations.len(),
                        violations
                            .iter()
                            .take(5)
                            .copied()
                            .collect::<Vec<_>>()
                            .join("\n")
                    )
                };

                VerificationResult {
                    check: VerificationCheck::FileConstraint { max_lines },
                    passed,
                    details,
                }
            }
            Err(e) => VerificationResult {
                check: VerificationCheck::FileConstraint { max_lines },
                passed: false,
                details: format!("Failed to check file constraints: {e}"),
            },
        }
    }
}

impl Default for Verifier {
    fn default() -> Self {
        Self::new()
    }
}
