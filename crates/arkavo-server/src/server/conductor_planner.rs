//! Multi-subtask execution path for the conductor.
//!
//! When a task is detected as complex (multi-step), this module decomposes it into
//! subtasks via `TaskPlanner` + `LlmIntentAnalyzer`, then executes each stage
//! through the tool loop. Falls back to the 1:1 path on any error.

use super::learning_bus::LearningBus;
use super::llm_intent_analyzer::LlmIntentAnalyzer;
use super::mcp_bridge::McpBridgeTool;
use super::tool_memory::ToolMemory;
use arkavo_hrm::{Conductor, store::InMemoryTaskStore};
use arkavo_mcp_tools::ToolRegistry;
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_tasks::{AgentRegistry, TaskPlanner};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Execute a complex task by decomposing it into planned subtasks.
///
/// Returns `Ok(final_text)` on success, or `Err` to signal the caller
/// should fall back to the 1:1 conductor path.
#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_with_plan(
    conductor: &Arc<Conductor<InMemoryTaskStore>>,
    router: &Arc<arkavo_router::Router>,
    mcp_registry: &Arc<McpRegistry>,
    task_content: &str,
    hrm_task_id: uuid::Uuid,
    model_hint: Option<&arkavo_router::ModelChoice>,
    learning_bus: Option<&Arc<LearningBus>>,
    tool_memory: Option<&Arc<tokio::sync::RwLock<ToolMemory>>>,
    system_prompt: Option<&str>,
    mesh_state: Option<&Arc<arkavo_mcp_mesh::MeshToolsState>>,
) -> Result<String, String> {
    // 1. Plan subtasks via LLM
    let analyzer = LlmIntentAnalyzer::new(router.clone());
    let planner = TaskPlanner::with_analyzer(Arc::new(AgentRegistry::new()), Box::new(analyzer));

    let (subtasks, execution_order, _deps) = planner
        .plan_subtasks(task_content)
        .await
        .map_err(|e| format!("Task planning failed: {e}"))?;

    if subtasks.len() <= 1 {
        return Err("Single subtask — use 1:1 path".into());
    }

    info!(
        subtasks = subtasks.len(),
        stages = execution_order.len(),
        "Executing planned decomposition"
    );

    // 2. Build tool registry (same setup as conductor.rs)
    let registry_arc = build_tool_registry(mcp_registry, mesh_state).await?;

    // 3. Execute each stage
    let mut stage_outputs: Vec<String> = Vec::new();
    let total_stages = execution_order.len();

    for (stage_idx, stage_ids) in execution_order.iter().enumerate() {
        let stage_subtasks: Vec<_> = stage_ids
            .iter()
            .filter_map(|id| subtasks.iter().find(|s| &s.id == id))
            .collect();

        info!(
            stage = stage_idx + 1,
            total = total_stages,
            subtasks = stage_subtasks.len(),
            "Executing stage"
        );

        // Register each subtask in HRM for dashboard visibility
        for st in &stage_subtasks {
            let _ = conductor
                .add_subtask(hrm_task_id, st.description.clone(), vec![])
                .await;
        }

        // Build context from prior stage outputs
        let prior_context = if stage_outputs.is_empty() {
            String::new()
        } else {
            format!("Previous results:\n{}\n\n", stage_outputs.join("\n---\n"))
        };

        if stage_subtasks.len() == 1 {
            let st = stage_subtasks[0];
            let content = format!("{prior_context}{}", st.description);
            let result = execute_subtask(
                router,
                &registry_arc,
                mcp_registry,
                &content,
                model_hint,
                learning_bus,
                tool_memory,
                system_prompt,
            )
            .await;
            stage_outputs.push(result);
        } else {
            // Parallel execution within a stage
            let mut handles = Vec::new();
            for st in &stage_subtasks {
                let content = format!("{prior_context}{}", st.description);
                let router = router.clone();
                let registry = registry_arc.clone();
                let mcp = mcp_registry.clone();
                let hint = model_hint.cloned();
                let bus = learning_bus.cloned();
                let mem = tool_memory.cloned();
                let sys = system_prompt.map(|s| s.to_string());

                handles.push(tokio::spawn(async move {
                    execute_subtask(
                        &router,
                        &registry,
                        &mcp,
                        &content,
                        hint.as_ref(),
                        bus.as_ref(),
                        mem.as_ref(),
                        sys.as_deref(),
                    )
                    .await
                }));
            }

            let results = futures::future::join_all(handles).await;
            for result in results {
                match result {
                    Ok(text) => stage_outputs.push(text),
                    Err(e) => {
                        warn!(error = %e, "Subtask join error");
                        stage_outputs.push(format!("[subtask failed: {e}]"));
                    }
                }
            }
        }

        // Update HRM progress
        let progress = ((stage_idx + 1) as f64) / (total_stages as f64);
        let _ = conductor.update_intra_progress(hrm_task_id, progress).await;
    }

    // 4. Synthesize final result
    let combined = stage_outputs.join("\n\n---\n\n");
    let final_result =
        match super::conductor_tool_loop::distill_with_small_model(router, &combined).await {
            Some(summary) => summary,
            None => combined,
        };

    info!(
        stages_completed = total_stages,
        output_len = final_result.len(),
        "Planned execution complete"
    );

    Ok(final_result)
}

async fn build_tool_registry(
    mcp_registry: &Arc<McpRegistry>,
    mesh_state: Option<&Arc<arkavo_mcp_mesh::MeshToolsState>>,
) -> Result<Arc<ToolRegistry>, String> {
    let mut tool_registry = ToolRegistry::empty();

    let mcp_tools = mcp_registry
        .list_all_tools()
        .await
        .map_err(|e| format!("Failed to list MCP tools: {e}"))?;

    let has_mcp_tools = !mcp_tools.is_empty();
    for tool in mcp_tools {
        let tool_name = tool.name.clone();
        let bridge = McpBridgeTool::new(mcp_registry.clone(), tool);
        tool_registry.register(&tool_name, Box::new(bridge));
    }

    if !has_mcp_tools {
        tool_registry.register(
            "filesystem_tools",
            Box::new(arkavo_mcp_tools::filesystem::FileSystemKit::new()),
        );
        tool_registry.register(
            "git_status",
            Box::new(arkavo_mcp_tools::git::GitStatusKit::new()),
        );
        tool_registry.register(
            "git_diff",
            Box::new(arkavo_mcp_tools::git::GitDiffKit::new()),
        );
        tool_registry.register("git_log", Box::new(arkavo_mcp_tools::git::GitLogKit::new()));
        tool_registry.register(
            "test_run",
            Box::new(arkavo_mcp_tools::test_runner::TestRunnerTool::new()),
        );
        tool_registry.register(
            "shell_exec",
            Box::new(arkavo_mcp_tools::shell_exec::ShellExecTool::new()),
        );
        tool_registry.register(
            "code_review",
            Box::new(arkavo_mcp_tools::code_review::CodeReviewTool::new()),
        );
    }

    if let Some(state) = mesh_state {
        arkavo_mcp_mesh::register_tools(&mut tool_registry, state.clone());
    }

    debug!(
        tools = tool_registry.list_tools().len(),
        "Tool registry built for planned execution"
    );

    Ok(Arc::new(tool_registry))
}

#[allow(clippy::too_many_arguments)]
async fn execute_subtask(
    router: &Arc<arkavo_router::Router>,
    registry_arc: &Arc<ToolRegistry>,
    mcp_registry: &Arc<McpRegistry>,
    task_content: &str,
    model_hint: Option<&arkavo_router::ModelChoice>,
    learning_bus: Option<&Arc<LearningBus>>,
    tool_memory: Option<&Arc<tokio::sync::RwLock<ToolMemory>>>,
    system_prompt: Option<&str>,
) -> String {
    let mut messages = Vec::new();
    if let Some(sys) = system_prompt {
        messages.push(arkavo_llm::Message::system(sys));
    }
    messages.push(arkavo_llm::Message::user(task_content));

    match super::conductor_tool_loop::run_tool_loop(
        router,
        registry_arc,
        mcp_registry,
        task_content,
        messages,
        model_hint,
        learning_bus,
        tool_memory,
        None, // compute_budget: planner doesn't enforce per-iteration budget
    )
    .await
    {
        Ok(result) => result.final_text,
        Err(e) => {
            warn!(error = %e, "Subtask execution failed");
            format!("[subtask error: {e}]")
        }
    }
}
