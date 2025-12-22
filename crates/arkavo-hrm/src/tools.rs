//! MCP Tools for HRM Conductor API
//!
//! Exposes the Conductor as MCP tools for agent orchestration:
//! - `hrm_create_task`: Create a new orchestrated task
//! - `hrm_get_status`: Get task state by ID
//! - `hrm_cancel_task`: Cancel a running task

use crate::conductor::Conductor;
use crate::schemas::{TaskBudget, TaskStatus};
use crate::store::InMemoryTaskStore;
use arkavo_mcp::ToolSchema;
use arkavo_mcp_tools::server::Tool;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Shared HRM state containing the Conductor
pub struct HrmToolsState {
    conductor: Conductor<InMemoryTaskStore>,
}

impl HrmToolsState {
    /// Create new HRM tools state with in-memory store
    pub fn new() -> Self {
        let store = InMemoryTaskStore::new();
        Self {
            conductor: Conductor::new(store),
        }
    }
}

impl Default for HrmToolsState {
    fn default() -> Self {
        Self::new()
    }
}

/// Tool for creating a new HRM task
pub struct CreateTaskTool {
    state: Arc<RwLock<HrmToolsState>>,
    schema: ToolSchema,
}

impl CreateTaskTool {
    pub fn new(state: Arc<RwLock<HrmToolsState>>) -> Self {
        Self {
            state,
            schema: ToolSchema {
                name: "hrm_create_task".to_string(),
                aliases: Some(vec!["create_task".to_string()]),
                description: "Create a new orchestrated task with HRM. The task will be \
                              decomposed into subtasks and executed with bounded contracts. \
                              Returns a task_id for tracking."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "objective": {
                            "type": "string",
                            "description": "The high-level objective for the task"
                        },
                        "max_cost_usd": {
                            "type": "number",
                            "description": "Maximum cost budget in USD (default: 10.0)"
                        },
                        "max_tokens": {
                            "type": "integer",
                            "description": "Maximum tokens to use (default: 100000)"
                        }
                    },
                    "required": ["objective"]
                }),
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct CreateTaskArgs {
    objective: String,
    #[serde(default)]
    max_cost_usd: Option<f64>,
    #[serde(default)]
    max_tokens: Option<u64>,
}

#[derive(Debug, Serialize)]
struct CreateTaskResponse {
    task_id: String,
    objective: String,
    status: String,
    budget: BudgetInfo,
}

#[derive(Debug, Serialize)]
struct BudgetInfo {
    max_cost_usd: f64,
    max_tokens: u64,
}

#[async_trait]
impl Tool for CreateTaskTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> arkavo_mcp_tools::Result<Value> {
        let args: CreateTaskArgs =
            serde_json::from_value(args).map_err(arkavo_mcp_tools::ToolError::Json)?;

        let mut budget = TaskBudget::default();
        if let Some(cost) = args.max_cost_usd {
            budget.max_cost_usd = cost;
        }
        if let Some(tokens) = args.max_tokens {
            budget.max_tokens = tokens;
        }

        let state = self.state.read().await;
        let task = state
            .conductor
            .create_task(args.objective.clone(), budget.clone())
            .await
            .map_err(|e| arkavo_mcp_tools::ToolError::Execution(e.to_string()))?;

        let response = CreateTaskResponse {
            task_id: task.id.to_string(),
            objective: task.objective,
            status: format!("{:?}", task.status),
            budget: BudgetInfo {
                max_cost_usd: budget.max_cost_usd,
                max_tokens: budget.max_tokens,
            },
        };

        serde_json::to_value(response).map_err(arkavo_mcp_tools::ToolError::Json)
    }
}

/// Tool for getting task status
pub struct GetStatusTool {
    state: Arc<RwLock<HrmToolsState>>,
    schema: ToolSchema,
}

impl GetStatusTool {
    pub fn new(state: Arc<RwLock<HrmToolsState>>) -> Self {
        Self {
            state,
            schema: ToolSchema {
                name: "hrm_get_status".to_string(),
                aliases: Some(vec!["get_task_status".to_string()]),
                description: "Get the current status of an HRM task including subtask \
                              progress, budget usage, and any errors."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "task_id": {
                            "type": "string",
                            "description": "The UUID of the task to query"
                        }
                    },
                    "required": ["task_id"]
                }),
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct GetStatusArgs {
    task_id: String,
}

#[derive(Debug, Serialize)]
struct TaskStatusResponse {
    task_id: String,
    objective: String,
    status: String,
    subtasks: Vec<SubtaskInfo>,
    budget_used: BudgetUsed,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize)]
struct SubtaskInfo {
    id: String,
    description: String,
    status: String,
    retry_count: u32,
}

#[derive(Debug, Serialize)]
struct BudgetUsed {
    cost_usd: f64,
    tokens: u64,
    remaining_cost_usd: f64,
    remaining_tokens: u64,
}

#[async_trait]
impl Tool for GetStatusTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> arkavo_mcp_tools::Result<Value> {
        let args: GetStatusArgs =
            serde_json::from_value(args).map_err(arkavo_mcp_tools::ToolError::Json)?;

        let task_id = Uuid::parse_str(&args.task_id)
            .map_err(|e| arkavo_mcp_tools::ToolError::Execution(format!("Invalid UUID: {e}")))?;

        let state = self.state.read().await;
        let task = state
            .conductor
            .get_task(task_id)
            .await
            .map_err(|e| arkavo_mcp_tools::ToolError::Execution(e.to_string()))?;

        let subtasks: Vec<SubtaskInfo> = task
            .subtasks
            .iter()
            .map(|s| SubtaskInfo {
                id: s.id.to_string(),
                description: s.description.clone(),
                status: format!("{:?}", s.status),
                retry_count: s.retry_count,
            })
            .collect();

        let response = TaskStatusResponse {
            task_id: task.id.to_string(),
            objective: task.objective,
            status: format!("{:?}", task.status),
            subtasks,
            budget_used: BudgetUsed {
                cost_usd: task.budget.spent_usd,
                tokens: task.budget.tokens_used,
                remaining_cost_usd: task.budget.remaining_cost(),
                remaining_tokens: task.budget.remaining_tokens(),
            },
            created_at: task.created_at.to_rfc3339(),
            updated_at: task.updated_at.to_rfc3339(),
        };

        serde_json::to_value(response).map_err(arkavo_mcp_tools::ToolError::Json)
    }
}

/// Tool for cancelling a task
pub struct CancelTaskTool {
    state: Arc<RwLock<HrmToolsState>>,
    schema: ToolSchema,
}

impl CancelTaskTool {
    pub fn new(state: Arc<RwLock<HrmToolsState>>) -> Self {
        Self {
            state,
            schema: ToolSchema {
                name: "hrm_cancel_task".to_string(),
                aliases: Some(vec!["cancel_task".to_string()]),
                description: "Cancel a running HRM task. All pending subtasks will be \
                              marked as cancelled."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "task_id": {
                            "type": "string",
                            "description": "The UUID of the task to cancel"
                        }
                    },
                    "required": ["task_id"]
                }),
            },
        }
    }
}

#[derive(Debug, Serialize)]
struct CancelTaskResponse {
    task_id: String,
    status: String,
    message: String,
}

#[async_trait]
impl Tool for CancelTaskTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> arkavo_mcp_tools::Result<Value> {
        let args: GetStatusArgs =
            serde_json::from_value(args).map_err(arkavo_mcp_tools::ToolError::Json)?;

        let task_id = Uuid::parse_str(&args.task_id)
            .map_err(|e| arkavo_mcp_tools::ToolError::Execution(format!("Invalid UUID: {e}")))?;

        let state = self.state.read().await;
        state
            .conductor
            .cancel_task(task_id)
            .await
            .map_err(|e| arkavo_mcp_tools::ToolError::Execution(e.to_string()))?;

        let response = CancelTaskResponse {
            task_id: task_id.to_string(),
            status: format!("{:?}", TaskStatus::Cancelled),
            message: "Task cancelled successfully".to_string(),
        };

        serde_json::to_value(response).map_err(arkavo_mcp_tools::ToolError::Json)
    }
}

/// Tool for listing all tasks
pub struct ListTasksTool {
    state: Arc<RwLock<HrmToolsState>>,
    schema: ToolSchema,
}

impl ListTasksTool {
    pub fn new(state: Arc<RwLock<HrmToolsState>>) -> Self {
        Self {
            state,
            schema: ToolSchema {
                name: "hrm_list_tasks".to_string(),
                aliases: Some(vec!["list_tasks".to_string()]),
                description: "List all HRM tasks with their current status.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "status_filter": {
                            "type": "string",
                            "description": "Optional: Filter by status (pending, running, completed, failed, cancelled)"
                        }
                    },
                    "required": []
                }),
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ListTasksArgs {
    #[serde(default)]
    status_filter: Option<String>,
}

#[derive(Debug, Serialize)]
struct ListTasksResponse {
    tasks: Vec<TaskSummary>,
    total: usize,
}

#[derive(Debug, Serialize)]
struct TaskSummary {
    task_id: String,
    objective: String,
    status: String,
    subtask_count: usize,
    created_at: String,
}

#[async_trait]
impl Tool for ListTasksTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> arkavo_mcp_tools::Result<Value> {
        let args: ListTasksArgs =
            serde_json::from_value(args).map_err(arkavo_mcp_tools::ToolError::Json)?;

        let state = self.state.read().await;
        let all_tasks = state.conductor.store().list_all().await;

        let status_filter = args.status_filter.map(|s| s.to_lowercase());

        let tasks: Vec<TaskSummary> = all_tasks
            .into_iter()
            .filter(|t| {
                if let Some(ref filter) = status_filter {
                    format!("{:?}", t.status).to_lowercase() == *filter
                } else {
                    true
                }
            })
            .map(|t| TaskSummary {
                task_id: t.id.to_string(),
                objective: t.objective.chars().take(80).collect(),
                status: format!("{:?}", t.status),
                subtask_count: t.subtasks.len(),
                created_at: t.created_at.to_rfc3339(),
            })
            .collect();

        let total = tasks.len();
        let response = ListTasksResponse { tasks, total };

        serde_json::to_value(response).map_err(arkavo_mcp_tools::ToolError::Json)
    }
}

/// Register all HRM tools with the tool registry
pub fn register_tools(
    registry: &mut arkavo_mcp_tools::ToolRegistry,
    state: Arc<RwLock<HrmToolsState>>,
) {
    registry.register(
        "hrm_create_task",
        Box::new(CreateTaskTool::new(state.clone())),
    );
    registry.register(
        "hrm_get_status",
        Box::new(GetStatusTool::new(state.clone())),
    );
    registry.register(
        "hrm_cancel_task",
        Box::new(CancelTaskTool::new(state.clone())),
    );
    registry.register("hrm_list_tasks", Box::new(ListTasksTool::new(state)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_task_tool() {
        let state = Arc::new(RwLock::new(HrmToolsState::new()));
        let tool = CreateTaskTool::new(state);

        let result = tool
            .execute(json!({
                "objective": "Test task",
                "max_cost_usd": 0.5
            }))
            .await
            .unwrap();

        assert!(result.get("task_id").is_some());
        assert_eq!(result["objective"], "Test task");
        assert_eq!(result["status"], "Pending");
    }

    #[tokio::test]
    async fn test_get_status_tool() {
        let state = Arc::new(RwLock::new(HrmToolsState::new()));

        // Create a task first
        let create_tool = CreateTaskTool::new(state.clone());
        let created = create_tool
            .execute(json!({"objective": "Status test"}))
            .await
            .unwrap();

        let task_id = created["task_id"].as_str().unwrap();

        // Now get status
        let status_tool = GetStatusTool::new(state);
        let result = status_tool
            .execute(json!({"task_id": task_id}))
            .await
            .unwrap();

        assert_eq!(result["task_id"], task_id);
        assert_eq!(result["objective"], "Status test");
    }

    #[tokio::test]
    async fn test_cancel_task_tool() {
        let state = Arc::new(RwLock::new(HrmToolsState::new()));

        // Create a task first
        let create_tool = CreateTaskTool::new(state.clone());
        let created = create_tool
            .execute(json!({"objective": "Cancel test"}))
            .await
            .unwrap();

        let task_id = created["task_id"].as_str().unwrap();

        // Cancel it
        let cancel_tool = CancelTaskTool::new(state.clone());
        let result = cancel_tool
            .execute(json!({"task_id": task_id}))
            .await
            .unwrap();

        assert_eq!(result["status"], "Cancelled");

        // Verify status changed
        let status_tool = GetStatusTool::new(state);
        let status = status_tool
            .execute(json!({"task_id": task_id}))
            .await
            .unwrap();

        assert_eq!(status["status"], "Cancelled");
    }

    #[tokio::test]
    async fn test_list_tasks_tool() {
        let state = Arc::new(RwLock::new(HrmToolsState::new()));

        // Create a few tasks
        let create_tool = CreateTaskTool::new(state.clone());
        create_tool
            .execute(json!({"objective": "Task 1"}))
            .await
            .unwrap();
        create_tool
            .execute(json!({"objective": "Task 2"}))
            .await
            .unwrap();

        // List all
        let list_tool = ListTasksTool::new(state);
        let result = list_tool.execute(json!({})).await.unwrap();

        assert_eq!(result["total"], 2);
        assert_eq!(result["tasks"].as_array().unwrap().len(), 2);
    }
}
