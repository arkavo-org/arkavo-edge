use arkavo_git::{GitManager, safety::RepoGuard};
use arkavo_llm::Message;
use arkavo_mcp_tools::ToolRegistry;
use arkavo_router::Router;
use async_trait::async_trait;
use std::sync::Arc;

use super::config::{ModelCapability, ModelInfo, SelectedModels, TaskConfig, TaskResult};
use super::planning::CollaborativePlanner;
use super::strategy::TaskStrategy;
use super::ui::TaskUI;
use crate::error::{Error, Result};

/// Local task execution strategy
///
/// Executes tasks using local and cloud models directly,
/// with collaborative 3-round planning.
pub struct LocalTaskStrategy {
    router: Option<Arc<Router>>,
    tool_registry: Option<Arc<ToolRegistry>>,
}

impl Default for LocalTaskStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalTaskStrategy {
    pub fn new() -> Self {
        Self {
            router: None,
            tool_registry: None,
        }
    }

    /// Set the router for LLM calls
    pub fn with_router(mut self, router: Arc<Router>) -> Self {
        self.router = Some(router);
        self
    }

    /// Set the tool registry for MCP tool execution
    pub fn with_tools(mut self, registry: Arc<ToolRegistry>) -> Self {
        self.tool_registry = Some(registry);
        self
    }

    /// Get or create the router
    async fn get_or_create_router(&self) -> Result<Arc<Router>> {
        if let Some(router) = &self.router {
            return Ok(router.clone());
        }
        Router::new().await.map(Arc::new).map_err(|e| Error::Model {
            operation: "create LLM router".to_string(),
            details: e.to_string(),
        })
    }

    /// Discover available local GGUF models in HuggingFace cache
    pub fn discover_local_models() -> Vec<ModelInfo> {
        let mut models = Vec::new();

        // Get HuggingFace cache directory
        let Some(hf_cache_dir) = dirs::home_dir().map(|d| d.join(".cache/huggingface/hub")) else {
            return models;
        };

        if !hf_cache_dir.exists() {
            return models;
        }

        // Scan for GGUF models in the cache
        let Ok(entries) = std::fs::read_dir(&hf_cache_dir) else {
            return models;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };

            // Check if it's a model directory
            if !dir_name.starts_with("models--") {
                continue;
            }

            // Look for GGUF files in snapshots
            let snapshots_dir = path.join("snapshots");
            if !snapshots_dir.exists() {
                continue;
            }

            let Ok(snapshot_entries) = std::fs::read_dir(&snapshots_dir) else {
                continue;
            };

            for snapshot in snapshot_entries.flatten() {
                let snapshot_path = snapshot.path();
                if !snapshot_path.is_dir() {
                    continue;
                }

                // Check for .gguf files
                let Ok(files) = std::fs::read_dir(&snapshot_path) else {
                    continue;
                };

                for file in files.flatten() {
                    let file_name_os = file.file_name();
                    let Some(file_name) = file_name_os.to_str() else {
                        continue;
                    };

                    if !file_name.ends_with(".gguf") {
                        continue;
                    }

                    let file_path = file.path();
                    let size_bytes = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
                    let size_gb = size_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                    let capability = Self::infer_capability(file_name, size_gb);

                    models.push(ModelInfo::local(
                        file_name.to_string(),
                        file_path,
                        size_gb,
                        capability,
                    ));
                }
            }
        }

        models
    }

    /// Detect available cloud LLM APIs
    pub fn detect_cloud_models() -> Vec<ModelInfo> {
        let mut models = Vec::new();

        // Check for Gemini
        if std::env::var("GEMINI_API_KEY").is_ok() {
            let model_id =
                std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-3.5-flash".to_string());
            models.push(ModelInfo::cloud("Gemini", "gemini", model_id));
        }

        // Check for OpenAI
        if std::env::var("OPENAI_API_KEY").is_ok() {
            models.push(ModelInfo::cloud("OpenAI", "openai", "gpt-4"));
        }

        // Check for DeepSeek
        if std::env::var("DEEPSEEK_API_KEY").is_ok() {
            models.push(ModelInfo::cloud("DeepSeek", "deepseek", "deepseek-chat"));
        }

        // Check for Anthropic
        if std::env::var("ANTHROPIC_API_KEY").is_ok() {
            let model_id = std::env::var("ANTHROPIC_MODEL")
                .unwrap_or_else(|_| "claude-sonnet-4-5-20250929".to_string());
            models.push(ModelInfo::cloud("Claude", "anthropic", model_id));
        }

        models
    }

    /// Infer model capability from name and size
    fn infer_capability(name: &str, size_gb: f64) -> ModelCapability {
        let name_lower = name.to_lowercase();

        // Check for large models first (7b+ parameters or large file size)
        if size_gb >= 8.0
            || name_lower.contains("-7b")
            || name_lower.contains("-8b")
            || name_lower.contains("-12b")
            || name_lower.contains("-14b")
            || name_lower.contains("7b-")
            || name_lower.contains("8b-")
            || name_lower.contains("12b-")
            || name_lower.contains("14b-")
        {
            ModelCapability::Large
        } else if size_gb < 2.0
            || name_lower.contains("270m")
            || name_lower.contains("-1b")
            || name_lower.contains("1b-")
        {
            ModelCapability::Small
        } else {
            // 2-7GB or 2b-4b parameter models
            ModelCapability::Medium
        }
    }

    /// Select appropriate models for task roles
    ///
    /// Returns SelectedModels with:
    /// - gather_model: Local model for info gathering
    /// - planning_model: Cloud model (preferred) or large local
    /// - verify_model: Same as gather_model
    pub fn select_models(
        local_models: &[ModelInfo],
        cloud_models: &[ModelInfo],
    ) -> Option<SelectedModels> {
        // Select cloud model: Gemini > Anthropic > OpenAI > DeepSeek
        let cloud = cloud_models
            .iter()
            .find(|m| m.provider == "gemini")
            .or_else(|| cloud_models.iter().find(|m| m.provider == "anthropic"))
            .or_else(|| cloud_models.iter().find(|m| m.provider == "openai"))
            .or_else(|| cloud_models.iter().find(|m| m.provider == "deepseek"))
            .cloned();

        // Select local model: prefer Medium > Large > Small
        // Medium (4B) is ideal: fast + capable for tool use
        let local = local_models
            .iter()
            .filter(|m| m.capability == ModelCapability::Medium)
            .max_by(|a, b| {
                a.size_gb
                    .partial_cmp(&b.size_gb)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .or_else(|| {
                local_models
                    .iter()
                    .filter(|m| m.capability == ModelCapability::Large)
                    .min_by(|a, b| {
                        a.size_gb
                            .partial_cmp(&b.size_gb)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            })
            .or(local_models.first())
            .cloned();

        // Need at least one model for planning
        let planning_model = cloud.clone().or_else(|| local.clone())?;
        let gather_model = local.clone().unwrap_or_else(|| planning_model.clone());
        let verify_model = gather_model.clone();

        Some(SelectedModels {
            gather_model,
            planning_model,
            verify_model,
        })
    }

    /// Display discovered models via UI
    fn show_models(&self, ui: &dyn TaskUI, local: &[ModelInfo], cloud: &[ModelInfo]) {
        ui.section("Available Models");
        ui.show_models(local, cloud);

        // Show individual models
        if !local.is_empty() {
            ui.status("Local Models:");
            for model in local {
                let cap_str = match model.capability {
                    ModelCapability::Small => "Small",
                    ModelCapability::Medium => "Medium",
                    ModelCapability::Large => "Large",
                };
                let size = model
                    .size_gb
                    .map(|s| format!("{s:.1} GB"))
                    .unwrap_or_default();
                ui.status(&format!("  - {} ({}) {}", model.name, cap_str, size));
            }
        }

        if !cloud.is_empty() {
            ui.status("Cloud Models:");
            for model in cloud {
                ui.status(&format!("  - {} ({})", model.name, model.model_id));
            }
        }
    }

    /// Display selected models for task
    fn show_selected(&self, ui: &dyn TaskUI, models: &SelectedModels) {
        ui.section("Selected for Task");
        ui.status(&format!(
            "Planning: {} ({})",
            models.planning_model.name, models.planning_model.provider
        ));
        ui.status(&format!(
            "Info Gathering: {} ({})",
            models.gather_model.name, models.gather_model.provider
        ));
        ui.status(&format!(
            "Verification: {} ({})",
            models.verify_model.name, models.verify_model.provider
        ));
    }
}

#[async_trait]
impl TaskStrategy for LocalTaskStrategy {
    async fn is_available(&self) -> bool {
        // Local strategy is available if we have at least one model
        let local = Self::discover_local_models();
        let cloud = Self::detect_cloud_models();
        !local.is_empty() || !cloud.is_empty()
    }

    async fn execute(
        &self,
        task: &str,
        config: &TaskConfig,
        ui: &dyn TaskUI,
    ) -> Result<TaskResult> {
        // Step 1: Discover available models
        let local_models = Self::discover_local_models();
        let cloud_models = Self::detect_cloud_models();

        self.show_models(ui, &local_models, &cloud_models);

        // Step 2: Select models for task
        let models = Self::select_models(&local_models, &cloud_models).ok_or_else(|| {
            ui.error("No models available");
            ui.error("Please either:");
            ui.error("  - Set GEMINI_API_KEY, OPENAI_API_KEY, or DEEPSEEK_API_KEY");
            ui.error(&format!(
                "  - Download a local model with: {}",
                arkavo_router::decision::ModelChoice::LocalQwen3
                    .download_hint()
                    .unwrap_or_default()
            ));
            Error::Model {
                operation: "select models for task".to_string(),
                details: "no local or cloud models available".to_string(),
            }
        })?;

        self.show_selected(ui, &models);

        // Step 3: Get or create router for LLM calls
        let router = self.get_or_create_router().await?;
        let planner = CollaborativePlanner::new(router.clone());

        // Step 4: Collaborative planning
        let tool_registry_ref = self.tool_registry.as_ref().map(|r| r.as_ref());
        let plan = planner
            .create_plan(task, &models, ui, tool_registry_ref)
            .await?;

        // Step 5: User approval
        ui.section("Step 2: Review Plan");
        ui.status(&format!("Plan content:\n{}", plan.plan_content));
        ui.status(&format!("Verification: {}", plan.verification_notes));

        if !config.auto_approve {
            if !ui.confirm("Execute this plan?").await {
                return Ok(TaskResult::failure("Task cancelled by user"));
            }
        }

        // Step 6: Execute plan with tools
        ui.section("Step 3: Execute Plan");
        let changes_made = self.execute_plan_with_tools(&planner, &plan, ui).await?;

        // Step 7: Git commit (if changes were made)
        let mut result = TaskResult::success(format!("Task completed: {}", plan.task));

        if changes_made {
            result = self.commit_changes(config, ui, result).await?;
        }

        ui.show_result(&result);
        Ok(result)
    }

    fn name(&self) -> &str {
        "local"
    }
}

impl LocalTaskStrategy {
    /// Execute the plan using MCP tools
    async fn execute_plan_with_tools(
        &self,
        planner: &CollaborativePlanner,
        plan: &super::planning::TaskPlan,
        ui: &dyn TaskUI,
    ) -> Result<bool> {
        ui.status("Executing plan...");

        let prompt = CollaborativePlanner::build_execution_prompt(&plan.task);
        let messages = vec![Message::user(format!(
            "{}\n\nBased on this plan:\n{}",
            prompt, plan.plan_content
        ))];

        let tool_registry_ref = self.tool_registry.as_ref().map(|r| r.as_ref());

        // Tool execution loop (max 20 iterations)
        let mut iterations = 0;
        let mut changes_made = false;
        let mut conversation = messages;

        while iterations < 20 {
            let response = planner
                .router()
                .route_with_tools("execute plan step", conversation.clone(), tool_registry_ref)
                .await
                .map_err(|e| Error::TaskExecution {
                    operation: "execute plan step via LLM".to_string(),
                    details: e.to_string(),
                })?;

            ui.progress(
                &format!(
                    "Iteration {}: {}",
                    iterations + 1,
                    if response.tool_calls.is_empty() {
                        "No tool calls"
                    } else {
                        "Processing tool calls"
                    }
                ),
                Some(((iterations + 1) * 5).min(100) as u8),
            );

            if response.tool_calls.is_empty() {
                ui.status(&format!("LLM response: {}", response.content));
                break;
            }

            // Add the assistant response (contains tool call markup) before results
            conversation.push(Message::assistant(&response.content));

            // Execute tool calls — results use Role::Tool for Jinja template compliance
            if let Some(registry) = &self.tool_registry {
                for tool_call in &response.tool_calls {
                    ui.status(&format!("Executing tool: {}", tool_call.tool_name));

                    let call_id = tool_call
                        .call_id
                        .clone()
                        .unwrap_or_else(|| format!("call_{}", iterations));

                    if let Some(tool) = registry.get(&tool_call.tool_name) {
                        match tool.execute(tool_call.arguments.clone()).await {
                            Ok(result) => {
                                ui.status(&format!("Tool result: {}", result));
                                changes_made = true;

                                conversation.push(Message::tool_result(
                                    result.to_string(),
                                    &call_id,
                                    &tool_call.tool_name,
                                ));
                            }
                            Err(e) => {
                                ui.warn(&format!("Tool {} failed: {}", tool_call.tool_name, e));
                                conversation.push(Message::tool_result(
                                    format!("Error: {e}"),
                                    &call_id,
                                    &tool_call.tool_name,
                                ));
                            }
                        }
                    } else {
                        ui.warn(&format!("Tool not found: {}", tool_call.tool_name));
                        conversation.push(Message::tool_result(
                            format!("Tool {} not found", tool_call.tool_name),
                            &call_id,
                            &tool_call.tool_name,
                        ));
                    }
                }
            } else {
                ui.warn("No tool registry available, skipping tool execution");
                break;
            }

            iterations += 1;
        }

        if iterations >= 20 {
            ui.warn("Reached maximum iteration limit");
        }

        ui.success("Plan execution completed");
        Ok(changes_made)
    }

    /// Commit changes with optional validation
    async fn commit_changes(
        &self,
        config: &TaskConfig,
        ui: &dyn TaskUI,
        mut result: TaskResult,
    ) -> Result<TaskResult> {
        ui.section("Step 4: Commit Changes");

        let git_manager = GitManager::new();
        let repo = match git_manager.open_repo(std::path::Path::new(".")) {
            Ok(r) => r,
            Err(e) => {
                ui.warn(&format!("Not a git repository: {}", e));
                return Ok(result);
            }
        };

        let mut guard = RepoGuard::new(&repo)?;

        if config.validate {
            ui.status("Running validation checks...");
            guard = guard.with_fmt_check().with_clippy_check();
        }

        let commit_result = guard.transaction(|repo| {
            if let Some(msg) = &config.commit_message {
                git_manager.add_all(repo)?;
                git_manager.commit_changes(repo, msg)
            } else {
                git_manager.auto_commit(repo)
            }
        });

        match commit_result {
            Ok(oid) => {
                result = result.with_commit(oid.to_string());
                ui.success(&format!("Committed: {}", oid));
            }
            Err(e) => {
                ui.error(&format!("Commit failed: {}", e));
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_executor::MockUI;
    use std::path::PathBuf;

    #[test]
    fn test_infer_capability_small() {
        assert_eq!(
            LocalTaskStrategy::infer_capability("qwen-270m.gguf", 0.3),
            ModelCapability::Small
        );
        assert_eq!(
            LocalTaskStrategy::infer_capability("model-1b.gguf", 1.5),
            ModelCapability::Small
        );
    }

    #[test]
    fn test_infer_capability_medium() {
        assert_eq!(
            LocalTaskStrategy::infer_capability("qwen-4b.gguf", 4.0),
            ModelCapability::Medium
        );
        assert_eq!(
            LocalTaskStrategy::infer_capability("model.gguf", 5.0),
            ModelCapability::Medium
        );
    }

    #[test]
    fn test_infer_capability_large() {
        assert_eq!(
            LocalTaskStrategy::infer_capability("qwen-12b.gguf", 12.0),
            ModelCapability::Large
        );
        assert_eq!(
            LocalTaskStrategy::infer_capability("model.gguf", 10.0),
            ModelCapability::Large
        );
    }

    #[test]
    fn test_select_models_cloud_preferred() {
        let local = vec![ModelInfo::local(
            "test.gguf",
            PathBuf::from("/test"),
            4.0,
            ModelCapability::Medium,
        )];
        let cloud = vec![ModelInfo::cloud("Gemini", "gemini", "gemini-3.5-flash")];

        let selected = LocalTaskStrategy::select_models(&local, &cloud).unwrap();

        assert_eq!(selected.planning_model.provider, "gemini");
        assert_eq!(selected.gather_model.provider, "local");
    }

    #[test]
    fn test_select_models_local_only() {
        let local = vec![
            ModelInfo::local(
                "small.gguf",
                PathBuf::from("/small"),
                0.5,
                ModelCapability::Small,
            ),
            ModelInfo::local(
                "medium.gguf",
                PathBuf::from("/medium"),
                4.0,
                ModelCapability::Medium,
            ),
        ];
        let cloud = vec![];

        let selected = LocalTaskStrategy::select_models(&local, &cloud).unwrap();

        // Medium preferred for all roles when no cloud
        assert!(selected.planning_model.name.contains("medium"));
        assert!(selected.gather_model.name.contains("medium"));
    }

    #[tokio::test]
    async fn test_local_strategy_is_available() {
        // Strategy should check for any available models
        let strategy = LocalTaskStrategy::new();
        // Result depends on actual environment, just test it doesn't panic
        let _ = strategy.is_available().await;
    }

    #[tokio::test]
    async fn test_local_strategy_execute() {
        let strategy = LocalTaskStrategy::new();
        let ui = MockUI::new().with_confirmations(vec![true]);
        let config = TaskConfig {
            auto_approve: true,
            ..Default::default()
        };

        // This test depends on whether models are available in the environment
        // We just verify it doesn't panic and handles gracefully
        let result = strategy.execute("test task", &config, &ui).await;

        // Either succeeds with models or fails with no-models error
        match result {
            Ok(r) => assert!(r.success || r.message.contains("completed")),
            Err(e) => assert!(e.to_string().contains("model") || e.to_string().contains("No")),
        }
    }
}
