//! EvoFabric execution mode for the Conductor
//!
//! Detects `[evofabric]` marker in task content and runs the AST-level code
//! evolution pipeline: LLM proposes an OpBundle, the Critic verifies it
//! (parse → apply → render → compile → test), and verified changes are
//! committed via git for atomic recoverability.

use super::learning_bus::LearningBus;
use arkavo_evofabric::{OpBundle, RustOp, crate_name_from_path, render_file};
use arkavo_hrm::{
    Conductor, burst::BurstResult, schemas::SubTaskBudget, schemas::TaskBudget,
    store::InMemoryTaskStore,
};
use std::sync::Arc;
use tracing::{info, warn};

/// Check if a task should be routed to evofabric mode
pub(super) fn is_evofabric_task(task_content: &str) -> bool {
    task_content.contains("[evofabric]")
}

/// Extract target file path and instruction from evofabric task content.
///
/// Format: `[evofabric] <relative/path/to/file.rs> <instruction>`
fn extract_target_and_instruction(task_content: &str) -> Option<(String, String)> {
    let after_marker = task_content.split_once("[evofabric]").map(|x| x.1)?.trim();

    let (path, instruction) = after_marker.split_once(char::is_whitespace)?;

    if path.is_empty() || instruction.trim().is_empty() {
        return None;
    }

    Some((path.to_string(), instruction.trim().to_string()))
}

/// Build the system prompt that instructs the LLM to produce a valid OpBundle JSON.
fn build_evofabric_prompt(source: &str, target_file: &str, instruction: &str) -> String {
    let mut prompt = String::with_capacity(source.len() + 2048);
    prompt.push_str("You are an EvoFabric code evolution agent. Your task is to propose a precise code modification as a structured JSON OpBundle.\n\n");
    prompt.push_str("## Target File\nPath: `");
    prompt.push_str(target_file);
    prompt.push_str("`\n\n## Current Source\n```rust\n");
    prompt.push_str(source);
    prompt.push_str("\n```\n\n## Instruction\n");
    prompt.push_str(instruction);
    prompt.push_str("\n\n## Response Format\nRespond with ONLY a JSON object. The schema:\n\n");
    prompt.push_str(
        r##"- "id": a zero UUID
- "originator": "evofabric-agent"
- "target_file": the file path
- "ops": array of operations (see below)
- "rationale": brief explanation
- "created_at": ISO timestamp
- "content_hash": empty string
- "status": {"state": "Proposed"}

## Scope Targeting

Each op has a "scope" that targets an AST node by semantic identity:
- {"kind": "Function", "name": "fn_name"} — free function
- {"kind": "Function", "name": "method", "within": {"kind": "ImplBlock", "type_name": "T"}} — method
- {"kind": "ImplBlock", "type_name": "T"} — impl block
- {"kind": "TypeDef", "name": "Name"} — struct/enum
- {"kind": "File"} — entire file

## Operations

1. {"op": "ReplaceFnBody", "scope": ..., "new_body": "{ ... }"} — replace function body
2. {"op": "InsertAfter", "scope": ..., "new_item": "fn new() {}"} — insert after target
3. {"op": "Remove", "scope": ...} — remove item
4. {"op": "ReplaceItem", "scope": ..., "new_item": "..."} — replace entire item
5. {"op": "AddAttribute", "scope": ..., "attribute": "#[inline]"} — add attribute
6. {"op": "AddUse", "path": "std::collections::HashMap"} — add use statement

## Rules
- new_body must be valid Rust wrapped in braces: { ... }
- new_item must be a complete valid Rust item
- Respond with ONLY the JSON, no other text
"##,
    );
    prompt
}

/// Execute the EvoFabric code evolution pipeline.
///
/// 1. Read target file source
/// 2. Ask LLM to propose an OpBundle
/// 3. Parse and apply the OpBundle to the AST
/// 4. Verify via compilation and tests in a temp workspace
/// 5. If verified: commit the change via git
#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_evofabric(
    conductor: &Arc<Conductor<InMemoryTaskStore>>,
    router: &Arc<arkavo_router::Router>,
    task_content: &str,
    _learning_bus: Option<&Arc<LearningBus>>,
    compute_budget: Option<&arkavo_budget::SharedComputeBudget>,
    model_hint: Option<&arkavo_router::ModelChoice>,
) -> Result<String, String> {
    let project_root =
        std::env::current_dir().map_err(|e| format!("Cannot determine project root: {e}"))?;
    let (target_file, instruction) = extract_target_and_instruction(task_content)
        .ok_or("Invalid format. Expected: [evofabric] <path> <instruction>")?;

    // Read the target source file
    let source_path = project_root.join(&target_file);
    let source = std::fs::read_to_string(&source_path)
        .map_err(|e| format!("Cannot read {target_file}: {e}"))?;

    let crate_name = crate_name_from_path(&target_file)
        .ok_or_else(|| format!("Cannot determine crate from path: {target_file}"))?
        .to_string();

    // Create HRM task
    let budget = TaskBudget::default();
    let hrm_task = conductor
        .create_task(format!("[evofabric] {target_file}"), budget)
        .await
        .map_err(|e| format!("Failed to create task: {e}"))?;

    let _subtask_budget = SubTaskBudget::evofabric();

    info!(
        task_id = %hrm_task.id,
        target = %target_file,
        "Starting EvoFabric code evolution"
    );

    // Build prompt and call LLM
    let prompt = build_evofabric_prompt(&source, &target_file, &instruction);
    let messages = vec![arkavo_llm::Message::user(prompt.clone())];

    // Check compute budget
    if let Some(cb) = compute_budget {
        let budget_guard = cb.read().await;
        if budget_guard.remaining_inferences == 0 {
            return Err("Compute budget exhausted".into());
        }
    }

    let response = router
        .route_with_tools_hinted(&prompt, messages, None, model_hint)
        .await
        .map_err(|e| format!("LLM inference failed: {e}"))?;

    // Consume inference from compute budget
    if let Some(cb) = compute_budget {
        let mut budget_guard = cb.write().await;
        budget_guard.consume_inference(0, 0.0);
    }

    // Parse the OpBundle from LLM output
    let bundle = OpBundle::from_json(&response.content).map_err(|e| {
        format!(
            "Failed to parse OpBundle from LLM output: {e}\n\nRaw output:\n{}",
            response.content
        )
    })?;

    info!(
        bundle_id = %bundle.id,
        ops = bundle.ops.len(),
        rationale = %bundle.rationale,
        "Parsed OpBundle from LLM"
    );

    // Apply the OpBundle to the AST (syn::File is !Send, keep before .await)
    let rendered = {
        let modified_ast = RustOp::apply_bundle(&source, &bundle)
            .map_err(|e| format!("AST operation failed: {e}"))?;
        render_file(&modified_ast)
    };

    // Verify in temp workspace
    let workspace = arkavo_evofabric::TempWorkspace::from_project(&project_root, &crate_name)
        .map_err(|e| format!("Failed to create temp workspace: {e}"))?;

    workspace
        .write_modified(&target_file, &rendered)
        .map_err(|e| format!("Failed to write modified source: {e}"))?;

    let verification = workspace
        .test(&crate_name)
        .await
        .map_err(|e| format!("Verification harness error: {e}"))?;

    if !verification.compile_ok {
        let msg = format!(
            "Compilation failed. OpBundle rejected.\n\nCompiler output:\n{}",
            verification.compile_stderr
        );
        warn!(bundle_id = %bundle.id, "EvoFabric bundle rejected: compile failure");

        // Record failure
        let subtask = conductor
            .add_subtask(hrm_task.id, "[evofabric] verify".into(), vec![])
            .await
            .map_err(|e| format!("Failed to add subtask: {e}"))?;
        let contract = conductor
            .create_contract(&subtask)
            .await
            .map_err(|e| format!("Failed to create contract: {e}"))?;
        let _ = conductor
            .record_result(
                hrm_task.id,
                subtask.id,
                BurstResult::failure(contract.id, msg.clone()),
            )
            .await;

        return Err(msg);
    }

    if !verification.test_ok {
        let msg = format!(
            "Tests failed. OpBundle rejected.\n\nTest output:\n{}",
            verification.test_stderr
        );
        warn!(bundle_id = %bundle.id, "EvoFabric bundle rejected: test failure");
        return Err(msg);
    }

    // Verified! Write the modified source and commit via git
    std::fs::write(&source_path, &rendered)
        .map_err(|e| format!("Failed to write {target_file}: {e}"))?;

    let commit_msg = format!("evofabric: {}", bundle.rationale);
    let git_result = tokio::process::Command::new("git")
        .args(["add", &target_file])
        .current_dir(&project_root)
        .output()
        .await
        .map_err(|e| format!("git add failed: {e}"))?;

    if git_result.status.success() {
        let _ = tokio::process::Command::new("git")
            .args(["commit", "-m", &commit_msg])
            .current_dir(&project_root)
            .output()
            .await;
    }

    // Record success in HRM
    let subtask = conductor
        .add_subtask(hrm_task.id, "[evofabric] apply".into(), vec![])
        .await
        .map_err(|e| format!("Failed to add subtask: {e}"))?;
    let contract = conductor
        .create_contract(&subtask)
        .await
        .map_err(|e| format!("Failed to create contract: {e}"))?;
    let burst_result = BurstResult::success(
        contract.id,
        serde_json::json!({
            "type": "evofabric",
            "target_file": target_file,
            "ops_count": bundle.ops.len(),
            "rationale": bundle.rationale,
            "bundle_id": bundle.id.to_string(),
        }),
    );
    let _ = conductor
        .record_result(hrm_task.id, subtask.id, burst_result)
        .await;

    info!(
        bundle_id = %bundle.id,
        target = %target_file,
        "EvoFabric bundle verified and applied"
    );

    Ok(format!(
        "EvoFabric: applied {} ops to `{}`\nRationale: {}\nBundle ID: {}",
        bundle.ops.len(),
        target_file,
        bundle.rationale,
        bundle.id
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_evofabric_task() {
        assert!(is_evofabric_task(
            "[evofabric] crates/foo/src/lib.rs optimize"
        ));
        assert!(is_evofabric_task("prefix [evofabric] suffix"));
        assert!(!is_evofabric_task("normal task"));
        assert!(!is_evofabric_task("[autoresearch] sweep"));
    }

    #[test]
    fn test_extract_target_and_instruction() {
        let (path, instr) =
            extract_target_and_instruction("[evofabric] crates/foo/src/lib.rs add #[inline]")
                .unwrap();
        assert_eq!(path, "crates/foo/src/lib.rs");
        assert_eq!(instr, "add #[inline]");
    }

    #[test]
    fn test_extract_target_missing_instruction() {
        assert!(extract_target_and_instruction("[evofabric] ").is_none());
    }

    #[test]
    fn test_extract_target_no_path() {
        assert!(extract_target_and_instruction("[evofabric]").is_none());
    }

    #[test]
    fn test_build_prompt_contains_source() {
        let prompt = build_evofabric_prompt("fn foo() {}", "src/lib.rs", "optimize");
        assert!(prompt.contains("fn foo()"));
        assert!(prompt.contains("src/lib.rs"));
        assert!(prompt.contains("optimize"));
        assert!(prompt.contains("ReplaceFnBody"));
    }

    #[test]
    fn test_subtask_budget_evofabric() {
        let budget = SubTaskBudget::evofabric();
        assert_eq!(budget.max_steps, 10);
        assert_eq!(budget.max_wall_time, std::time::Duration::from_secs(300));
    }
}
