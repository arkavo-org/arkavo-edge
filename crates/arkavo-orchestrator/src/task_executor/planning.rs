use super::config::SelectedModels;
use super::ui::TaskUI;
use crate::error::{Error, Result};
use arkavo_llm::Message;
use arkavo_mcp_tools::ToolRegistry;
use arkavo_router::Router;
use std::sync::Arc;

/// Collaborative planner implementing 3-round planning pattern
///
/// Round 1: Local model gathers information (filesystem, git status)
/// Round 2: Cloud model creates detailed plan based on findings
/// Round 3: Local model verifies plan correctness
pub struct CollaborativePlanner {
    router: Arc<Router>,
}

impl CollaborativePlanner {
    pub fn new(router: Arc<Router>) -> Self {
        Self { router }
    }

    pub fn router(&self) -> &Arc<Router> {
        &self.router
    }

    /// Create a plan through 3-round collaboration
    pub async fn create_plan(
        &self,
        task: &str,
        _models: &SelectedModels,
        ui: &dyn TaskUI,
        tool_registry: Option<&ToolRegistry>,
    ) -> Result<TaskPlan> {
        ui.section("Step 1: Collaborative Planning");

        // Round 1: Local model gathers information
        ui.progress("Local Agent gathering information...", Some(33));
        let gather_result = self.round_gather(task, ui, tool_registry).await?;

        // Round 2: Cloud model creates plan
        ui.progress("Planning Agent creating plan...", Some(66));
        let plan_content = self
            .round_plan(task, &gather_result, ui, tool_registry)
            .await?;

        // Round 3: Local model verifies
        ui.progress("Local Agent verifying plan...", Some(100));
        let verified = self
            .round_verify(&plan_content, ui, tool_registry)
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
        ui: &dyn TaskUI,
        tool_registry: Option<&ToolRegistry>,
    ) -> Result<String> {
        ui.status(&format!(
            "[Local Agent] Gathering information for: {}",
            task
        ));

        let prompt = Self::build_gather_prompt(task);
        let messages = vec![Message::user(prompt)];

        let response = self
            .router
            .route_with_tools(
                &format!("gather context for: {}", task),
                messages,
                tool_registry,
            )
            .await
            .map_err(|e| Error::Config(format!("LLM gather failed: {}", e)))?;

        Ok(response.content)
    }

    /// Round 2: Create detailed plan based on findings
    async fn round_plan(
        &self,
        task: &str,
        gather_result: &str,
        ui: &dyn TaskUI,
        tool_registry: Option<&ToolRegistry>,
    ) -> Result<String> {
        ui.status("[Planning Agent] Creating detailed plan...");

        let prompt = Self::build_plan_prompt(task, gather_result);
        let messages = vec![Message::user(prompt)];

        let response = self
            .router
            .route_with_tools(&format!("create plan for: {}", task), messages, tool_registry)
            .await
            .map_err(|e| Error::Config(format!("LLM planning failed: {}", e)))?;

        Ok(response.content)
    }

    /// Round 3: Verify plan correctness
    async fn round_verify(
        &self,
        plan_content: &str,
        ui: &dyn TaskUI,
        tool_registry: Option<&ToolRegistry>,
    ) -> Result<String> {
        ui.status("[Local Agent] Verifying plan...");

        let prompt = Self::build_verify_prompt(plan_content);
        let messages = vec![Message::user(prompt)];

        let response = self
            .router
            .route_with_tools("verify plan correctness", messages, tool_registry)
            .await
            .map_err(|e| Error::Config(format!("LLM verification failed: {}", e)))?;

        Ok(response.content)
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
