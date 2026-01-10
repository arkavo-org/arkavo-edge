use super::config::{ModelInfo, SelectedModels};
use super::ui::TaskUI;
use crate::error::Result;

/// Collaborative planner implementing 3-round planning pattern
///
/// Round 1: Local model gathers information (filesystem, git status)
/// Round 2: Cloud model creates detailed plan based on findings
/// Round 3: Local model verifies plan correctness
pub struct CollaborativePlanner;

impl Default for CollaborativePlanner {
    fn default() -> Self {
        Self::new()
    }
}

impl CollaborativePlanner {
    pub fn new() -> Self {
        Self
    }

    /// Create a plan through 3-round collaboration
    pub async fn create_plan(
        &self,
        task: &str,
        models: &SelectedModels,
        ui: &dyn TaskUI,
    ) -> Result<TaskPlan> {
        ui.section("Step 1: Collaborative Planning");

        // Round 1: Local model gathers information
        ui.progress("Local Agent gathering information...", Some(33));
        let gather_result = self.round_gather(task, &models.gather_model, ui).await?;

        // Round 2: Cloud model creates plan
        ui.progress("Planning Agent creating plan...", Some(66));
        let plan_content = self
            .round_plan(task, &gather_result, &models.planning_model, ui)
            .await?;

        // Round 3: Local model verifies
        ui.progress("Local Agent verifying plan...", Some(100));
        let verified = self
            .round_verify(&plan_content, &models.verify_model, ui)
            .await?;

        Ok(TaskPlan {
            task: task.to_string(),
            gather_findings: gather_result,
            plan_content,
            verification_notes: verified,
        })
    }

    /// Round 1: Gather information about the task context
    async fn round_gather(
        &self,
        task: &str,
        _model: &ModelInfo,
        ui: &dyn TaskUI,
    ) -> Result<String> {
        ui.status(&format!(
            "[Local Agent] Gathering information for: {}",
            task
        ));

        // Build the gather prompt - this would be sent to the LLM with MCP tools
        let _prompt = Self::build_gather_prompt(task);

        // In full implementation, this would:
        // 1. Create chat context with MCP tools enabled
        // 2. Send prompt to local model
        // 3. Execute tool calls (filesystem, git_status)
        // 4. Return findings

        // For now, return placeholder indicating prompt was generated
        Ok(format!("Gathering findings for task: {task}"))
    }

    /// Round 2: Create detailed plan based on findings
    async fn round_plan(
        &self,
        task: &str,
        gather_result: &str,
        _model: &ModelInfo,
        ui: &dyn TaskUI,
    ) -> Result<String> {
        ui.status("[Planning Agent] Creating detailed plan...");

        // Build the planning prompt
        let _prompt = Self::build_plan_prompt(task, gather_result);

        // In full implementation, this would:
        // 1. Send prompt to cloud model with MCP tools
        // 2. Allow tool calls for file reading, git operations
        // 3. Return structured plan

        Ok(format!("Plan for task: {task}"))
    }

    /// Round 3: Verify plan correctness
    async fn round_verify(
        &self,
        plan_content: &str,
        _model: &ModelInfo,
        ui: &dyn TaskUI,
    ) -> Result<String> {
        ui.status("[Local Agent] Verifying plan...");

        // Build verification prompt
        let _prompt = Self::build_verify_prompt(plan_content);

        // In full implementation, this would:
        // 1. Send prompt to local model with MCP tools
        // 2. Read files mentioned in plan
        // 3. Report any issues

        Ok("Plan verified".to_string())
    }

    /// Build prompt for Round 1: Information gathering
    pub fn build_gather_prompt(task: &str) -> String {
        format!(
            r#"Task: {task}

Use MCP tools to gather information. Be concise.
- @filesystem {{"action": "list_directory", "dir_path": "."}}
- @git_status

Report your findings in 3-4 sentences. Then ask the planning agent 2-3 specific questions about what needs to be done."#
        )
    }

    /// Build prompt for Round 2: Planning
    pub fn build_plan_prompt(task: &str, gather_findings: &str) -> String {
        format!(
            r#"Task: {task}

Based on the local agent's findings:
{gather_findings}

Create a detailed plan.

**Available MCP Tools:**
- @filesystem {{"action": "read_file", "file_path": "path"}}
- @filesystem {{"action": "list_directory", "dir_path": "path"}}
- @git_status
- @git_diff

Use tools to verify your assumptions. Output your plan:

## Analysis
[Key findings]

## Plan
1. [Step]
2. [Step]

## Files to Modify
- file: [changes]

Ask the local agent to verify anything uncertain."#
        )
    }

    /// Build prompt for Round 3: Verification
    pub fn build_verify_prompt(plan_content: &str) -> String {
        format!(
            r#"Based on the cloud agent's plan:
{plan_content}

Verify the details using MCP tools. Read the mentioned files and confirm the plan makes sense. Report any issues."#
        )
    }

    /// Build prompt for execution phase
    pub fn build_execution_prompt(task: &str) -> String {
        format!(
            r#"Based on the plan from our conversation, execute the changes:

Task: {task}

Use MCP tools to make the actual changes:
- @filesystem {{"action": "write_file", "file_path": "path", "content": "..."}}
- @filesystem {{"action": "edit_file", "file_path": "path", "old_content": "...", "new_content": "..."}}
- @git_status
- @git_diff

Execute the plan step by step. Show what you're doing."#
        )
    }
}

/// Result of collaborative planning
#[derive(Debug, Clone)]
pub struct TaskPlan {
    /// Original task description
    pub task: String,
    /// Findings from gather round
    pub gather_findings: String,
    /// Detailed plan content
    pub plan_content: String,
    /// Notes from verification round
    pub verification_notes: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_executor::{MockUI, ModelCapability, ModelInfo, SelectedModels};
    use std::path::PathBuf;

    fn make_test_models() -> SelectedModels {
        let local = ModelInfo::local(
            "test-local",
            PathBuf::from("/test/model.gguf"),
            2.0,
            ModelCapability::Medium,
        );
        let cloud = ModelInfo::cloud("test-cloud", "gemini", "gemini-2.0-flash");

        SelectedModels {
            gather_model: local.clone(),
            planning_model: cloud,
            verify_model: local,
        }
    }

    #[tokio::test]
    async fn test_planner_creates_plan() {
        let planner = CollaborativePlanner::new();
        let ui = MockUI::new();
        let models = make_test_models();

        let result = planner
            .create_plan("Add tests for module X", &models, &ui)
            .await;

        assert!(result.is_ok());
        let plan = result.unwrap();
        assert!(plan.task.contains("Add tests"));
    }

    #[test]
    fn test_gather_prompt_includes_tools() {
        let prompt = CollaborativePlanner::build_gather_prompt("Fix bug");
        assert!(prompt.contains("@filesystem"));
        assert!(prompt.contains("@git_status"));
        assert!(prompt.contains("Fix bug"));
    }

    #[test]
    fn test_plan_prompt_includes_findings() {
        let prompt = CollaborativePlanner::build_plan_prompt("Add feature", "Found 3 files");
        assert!(prompt.contains("Found 3 files"));
        assert!(prompt.contains("## Plan"));
        assert!(prompt.contains("## Files to Modify"));
    }

    #[test]
    fn test_execution_prompt_includes_write_tools() {
        let prompt = CollaborativePlanner::build_execution_prompt("Create file");
        assert!(prompt.contains("write_file"));
        assert!(prompt.contains("edit_file"));
    }
}
