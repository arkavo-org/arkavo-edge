use crate::checks::{CheckResult, VerificationCheck, VerificationInput};
use crate::evidence::{CheckSeverity, VerificationEvidence};
use arkavo_evofabric::{OpBundle, RustOp, TempWorkspace, crate_name_from_path, render_file};
use async_trait::async_trait;
use std::path::PathBuf;
use std::time::Instant;

/// Verification check that compiles and tests Rust code changes from OpBundles.
///
/// Applicable when `task_category == "evofabric"` and `context` contains an
/// `op_bundle` key. Runs the full pipeline: parse → apply → render → compile → test.
pub struct CodeVerificationCheck {
    project_root: PathBuf,
}

impl CodeVerificationCheck {
    pub fn new(project_root: PathBuf) -> Self {
        Self { project_root }
    }
}

#[async_trait]
impl VerificationCheck for CodeVerificationCheck {
    fn id(&self) -> &'static str {
        "code_verification"
    }

    fn priority(&self) -> u32 {
        25
    }

    fn is_applicable(&self, input: &VerificationInput) -> bool {
        input.task_category.as_deref() == Some("evofabric")
            && input
                .context
                .as_ref()
                .is_some_and(|ctx| ctx.get("op_bundle").is_some())
    }

    async fn verify(&self, input: &VerificationInput) -> CheckResult {
        let start = Instant::now();

        let ctx = match &input.context {
            Some(ctx) => ctx,
            None => return CheckResult::Skip("no context".into()),
        };

        let bundle_json = match ctx.get("op_bundle") {
            Some(v) => v.to_string(),
            None => return CheckResult::Skip("no op_bundle in context".into()),
        };

        // Parse the OpBundle
        let bundle = match OpBundle::from_json(&bundle_json) {
            Ok(b) => b,
            Err(e) => {
                return CheckResult::Fail(
                    VerificationEvidence::failed(
                        self.id(),
                        &format!("failed to parse OpBundle: {e}"),
                        CheckSeverity::Critical,
                        start.elapsed().as_micros() as u64,
                    )
                    .with_details(serde_json::json!({ "raw_input": bundle_json })),
                );
            }
        };

        // Read the target file
        let target_path = self.project_root.join(&bundle.target_file);
        let source = match std::fs::read_to_string(&target_path) {
            Ok(s) => s,
            Err(e) => {
                return CheckResult::Fail(VerificationEvidence::failed(
                    self.id(),
                    &format!("cannot read {}: {e}", bundle.target_file),
                    CheckSeverity::Critical,
                    start.elapsed().as_micros() as u64,
                ));
            }
        };

        // Apply the OpBundle to the AST and render (syn::File is !Send,
        // so all AST work must complete before any .await)
        let rendered = match RustOp::apply_bundle(&source, &bundle) {
            Ok(ast) => render_file(&ast),
            Err(e) => {
                return CheckResult::Fail(VerificationEvidence::failed(
                    self.id(),
                    &format!("AST operation failed: {e}"),
                    CheckSeverity::Critical,
                    start.elapsed().as_micros() as u64,
                ));
            }
        };

        // Determine the crate name
        let crate_name = match crate_name_from_path(&bundle.target_file) {
            Some(name) => name.to_string(),
            None => {
                return CheckResult::Fail(VerificationEvidence::failed(
                    self.id(),
                    &format!("cannot determine crate from path: {}", bundle.target_file),
                    CheckSeverity::Critical,
                    start.elapsed().as_micros() as u64,
                ));
            }
        };

        // Create temp workspace and compile
        let workspace = match TempWorkspace::from_project(&self.project_root, &crate_name) {
            Ok(ws) => ws,
            Err(e) => {
                return CheckResult::Fail(VerificationEvidence::failed(
                    self.id(),
                    &format!("failed to create temp workspace: {e}"),
                    CheckSeverity::Critical,
                    start.elapsed().as_micros() as u64,
                ));
            }
        };

        if let Err(e) = workspace.write_modified(&bundle.target_file, &rendered) {
            return CheckResult::Fail(VerificationEvidence::failed(
                self.id(),
                &format!("failed to write modified source: {e}"),
                CheckSeverity::Critical,
                start.elapsed().as_micros() as u64,
            ));
        }

        // Run cargo check + test
        let result = match workspace.test(&crate_name).await {
            Ok(r) => r,
            Err(e) => {
                return CheckResult::Fail(VerificationEvidence::failed(
                    self.id(),
                    &format!("verification harness error: {e}"),
                    CheckSeverity::Critical,
                    start.elapsed().as_micros() as u64,
                ));
            }
        };

        let latency = start.elapsed().as_micros() as u64;

        if !result.compile_ok {
            return CheckResult::Fail(
                VerificationEvidence::failed(
                    self.id(),
                    &format!("compilation failed:\n{}", result.compile_stderr),
                    CheckSeverity::Critical,
                    latency,
                )
                .with_details(serde_json::json!({
                    "stage": "compile",
                    "stderr": result.compile_stderr,
                    "target_file": bundle.target_file,
                    "crate": crate_name,
                })),
            );
        }

        if !result.test_ok {
            return CheckResult::Fail(
                VerificationEvidence::failed(
                    self.id(),
                    &format!("tests failed:\n{}", result.test_stderr),
                    CheckSeverity::Error,
                    latency,
                )
                .with_details(serde_json::json!({
                    "stage": "test",
                    "stderr": result.test_stderr,
                    "target_file": bundle.target_file,
                    "crate": crate_name,
                })),
            );
        }

        CheckResult::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_llm::ProviderResponse;

    fn dummy_input() -> VerificationInput {
        VerificationInput::new(
            "test".into(),
            ProviderResponse {
                content: String::new(),
                reasoning_content: None,
                tool_calls: vec![],
                finish_reason: None,
                inference_timing: None,
                quality_gate_retries: 0,
                ..Default::default()
            },
            vec![],
        )
    }

    #[test]
    fn skip_when_not_evofabric() {
        let check = CodeVerificationCheck::new(PathBuf::from("/tmp"));
        let input = dummy_input();
        assert!(!check.is_applicable(&input));
    }

    #[test]
    fn skip_when_no_op_bundle() {
        let check = CodeVerificationCheck::new(PathBuf::from("/tmp"));
        let input = dummy_input().with_category("evofabric");
        assert!(!check.is_applicable(&input));
    }

    #[test]
    fn applicable_with_category_and_bundle() {
        let check = CodeVerificationCheck::new(PathBuf::from("/tmp"));
        let input = dummy_input()
            .with_category("evofabric")
            .with_context(serde_json::json!({ "op_bundle": {} }));
        assert!(check.is_applicable(&input));
    }
}
