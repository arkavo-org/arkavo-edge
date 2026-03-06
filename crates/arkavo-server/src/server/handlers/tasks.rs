use arkavo_hrm::{Conductor, store::InMemoryTaskStore};
use arkavo_protocol::metrics::{MetricsCollector, RpcTimer};
use arkavo_protocol::rate_limit::RateLimiter;
use arkavo_protocol::types::{
    HrmTaskSummary, Message, TaskCancelRequest, TaskCancelResponse, TaskGetRequest,
    TaskGetResponse, TaskListRequest, TaskListResponse, TaskStatus,
};
use arkavo_tasks::task_executor::TaskExecutor;
use arkavo_tasks::task_store::TaskStore;
use jsonrpsee::types::ErrorObjectOwned;
use std::sync::Arc;

pub async fn handle_tasks_get(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    task_store: &Arc<dyn TaskStore>,
    request: TaskGetRequest,
) -> Result<TaskGetResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("tasks/get".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    let task_id = match uuid::Uuid::parse_str(&request.task_id) {
        Ok(id) => id,
        Err(_) => {
            timer.error();
            return Err(ErrorObjectOwned::owned(
                -32602,
                "Invalid task ID",
                Some("Task ID must be a valid UUID".to_string()),
            ));
        }
    };

    match task_store.get_task(&task_id).await {
        Ok(Some(task)) => {
            let result = if task.status == TaskStatus::Completed {
                // Try dedicated task_results table first, fall back to Task.result field
                task_store
                    .get_task_result(&task_id)
                    .await
                    .ok()
                    .flatten()
                    .or_else(|| task.result.clone())
                    .and_then(|v| serde_json::from_value::<Message>(v).ok())
            } else {
                None
            };

            let response = TaskGetResponse {
                task_id: task.id.to_string(),
                status: task.status,
                result,
                error: task.error,
                progress: task.progress,
            };
            timer.success();
            Ok(response)
        }
        Ok(None) => {
            timer.error();
            Err(ErrorObjectOwned::owned(
                -32602,
                "Task not found",
                Some(format!("No task found with ID: {}", request.task_id)),
            ))
        }
        Err(e) => {
            timer.error();
            Err(ErrorObjectOwned::owned(
                -32603,
                "Failed to retrieve task",
                Some(format!("Error: {e}")),
            ))
        }
    }
}

pub async fn handle_tasks_cancel(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    task_store: &Arc<dyn TaskStore>,
    task_executor: &Arc<TaskExecutor>,
    request: TaskCancelRequest,
) -> Result<TaskCancelResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("tasks/cancel".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    let task_id = match uuid::Uuid::parse_str(&request.task_id) {
        Ok(id) => id,
        Err(_) => {
            timer.error();
            return Err(ErrorObjectOwned::owned(
                -32602,
                "Invalid task ID",
                Some("Task ID must be a valid UUID".to_string()),
            ));
        }
    };

    match task_executor
        .update_task_status(&task_id, TaskStatus::Canceled)
        .await
    {
        Ok(()) => {
            let response = TaskCancelResponse {
                success: true,
                status: TaskStatus::Canceled,
                message: request
                    .reason
                    .or_else(|| Some("Task cancelled successfully".to_string())),
            };
            timer.success();
            Ok(response)
        }
        Err(e) => {
            let current_status = task_store
                .get_task(&task_id)
                .await
                .ok()
                .flatten()
                .map(|t| t.status)
                .unwrap_or(TaskStatus::Failed);

            let response = TaskCancelResponse {
                success: false,
                status: current_status,
                message: Some(format!("Failed to cancel task: {e}")),
            };
            timer.success();
            Ok(response)
        }
    }
}

pub async fn handle_tasks_list(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    conductor: &Arc<Conductor<InMemoryTaskStore>>,
    request: TaskListRequest,
) -> Result<TaskListResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("tasks/list".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    let all_tasks = conductor.store().list_all().await;
    let limit = request.limit.unwrap_or(50) as usize;

    let mut tasks: Vec<HrmTaskSummary> = all_tasks
        .iter()
        .filter(|t| {
            request
                .since
                .as_ref()
                .is_none_or(|since| t.updated_at.to_rfc3339() > *since)
        })
        .rev()
        .take(limit)
        .map(|t| {
            let objective = if t.objective.len() > 200 {
                format!("{}...", &t.objective[..197])
            } else {
                t.objective.clone()
            };
            let quality_score = t
                .subtasks
                .iter()
                .rev()
                .find_map(|s| s.result.as_ref()?.quality_score);
            HrmTaskSummary {
                id: t.id.to_string(),
                objective,
                status: format!("{:?}", t.status),
                created_at: t.created_at.to_rfc3339(),
                updated_at: t.updated_at.to_rfc3339(),
                progress: t.progress(),
                tool_calls: vec![],
                quality_score,
            }
        })
        .collect();

    tasks.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    let total_count = tasks.len() as u32;
    timer.success();
    Ok(TaskListResponse { tasks, total_count })
}
