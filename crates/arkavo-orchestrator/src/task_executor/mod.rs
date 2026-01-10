mod config;
mod local_strategy;
mod mesh_strategy;
mod planning;
mod strategy;
mod ui;

pub use config::{ModelCapability, ModelInfo, SelectedModels, TaskConfig, TaskResult};
pub use local_strategy::LocalTaskStrategy;
pub use mesh_strategy::MeshTaskStrategy;
pub use planning::{CollaborativePlanner, TaskPlan};
pub use strategy::TaskStrategy;
pub use ui::{MessageType, MockUI, TaskUI};

use crate::error::Result;
use std::sync::Arc;

/// Task executor that coordinates strategy selection and planning
///
/// Supports both local and mesh execution paths with automatic
/// fallback when mesh is unavailable.
pub struct TaskExecutor {
    local_strategy: Option<Arc<dyn TaskStrategy>>,
    mesh_strategy: Option<Arc<dyn TaskStrategy>>,
}

impl Default for TaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskExecutor {
    /// Create a new task executor
    pub fn new() -> Self {
        Self {
            local_strategy: None,
            mesh_strategy: None,
        }
    }

    /// Set the local execution strategy
    pub fn with_local_strategy(mut self, strategy: Arc<dyn TaskStrategy>) -> Self {
        self.local_strategy = Some(strategy);
        self
    }

    /// Set the mesh execution strategy
    pub fn with_mesh_strategy(mut self, strategy: Arc<dyn TaskStrategy>) -> Self {
        self.mesh_strategy = Some(strategy);
        self
    }

    /// Main entry point - runs task with automatic strategy selection
    ///
    /// Strategy selection:
    /// 1. If mesh_only: Use mesh, fail if unavailable
    /// 2. If use_local_only: Use local directly
    /// 3. Default: Try mesh first, fall back to local
    pub async fn run_task(
        &self,
        task: &str,
        config: &TaskConfig,
        ui: &dyn TaskUI,
    ) -> Result<TaskResult> {
        ui.section("Task: Plan and Execute");
        ui.status(&format!("Task: {task}"));

        // Try mesh execution unless local-only
        if !config.use_local_only {
            if let Some(mesh) = &self.mesh_strategy {
                if mesh.is_available().await {
                    match mesh.execute(task, config, ui).await {
                        Ok(result) => {
                            ui.success("Task completed via mesh network");
                            return Ok(result);
                        }
                        Err(e) => {
                            if config.mesh_only {
                                ui.error("Mesh execution failed and --mesh-only specified");
                                return Err(e);
                            }
                            ui.warn(&format!("Mesh unavailable: {}, falling back to local", e));
                        }
                    }
                } else if config.mesh_only {
                    ui.error("No mesh agents available and --mesh-only specified");
                    return Ok(TaskResult::failure("No mesh agents available"));
                }
            } else if config.mesh_only {
                ui.error("Mesh strategy not configured and --mesh-only specified");
                return Ok(TaskResult::failure("Mesh strategy not configured"));
            }
        }

        // Fall back to local execution
        if let Some(local) = &self.local_strategy {
            local.execute(task, config, ui).await
        } else {
            ui.error("No execution strategy available");
            Ok(TaskResult::failure("No execution strategy available"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockStrategy {
        name: &'static str,
        available: bool,
        result: TaskResult,
    }

    #[async_trait::async_trait]
    impl TaskStrategy for MockStrategy {
        async fn is_available(&self) -> bool {
            self.available
        }

        async fn execute(
            &self,
            _task: &str,
            _config: &TaskConfig,
            ui: &dyn TaskUI,
        ) -> Result<TaskResult> {
            ui.status(&format!("Executing via {}", self.name));
            Ok(self.result.clone())
        }

        fn name(&self) -> &str {
            self.name
        }
    }

    #[tokio::test]
    async fn test_executor_uses_mesh_first() {
        let ui = MockUI::new();
        let executor = TaskExecutor::new()
            .with_mesh_strategy(Arc::new(MockStrategy {
                name: "mesh",
                available: true,
                result: TaskResult::success("mesh done"),
            }))
            .with_local_strategy(Arc::new(MockStrategy {
                name: "local",
                available: true,
                result: TaskResult::success("local done"),
            }));

        let result = executor
            .run_task("test", &TaskConfig::default(), &ui)
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.message, "mesh done");
    }

    #[tokio::test]
    async fn test_executor_falls_back_to_local() {
        let ui = MockUI::new();
        let executor = TaskExecutor::new()
            .with_mesh_strategy(Arc::new(MockStrategy {
                name: "mesh",
                available: false,
                result: TaskResult::success("mesh done"),
            }))
            .with_local_strategy(Arc::new(MockStrategy {
                name: "local",
                available: true,
                result: TaskResult::success("local done"),
            }));

        let result = executor
            .run_task("test", &TaskConfig::default(), &ui)
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.message, "local done");
    }

    #[tokio::test]
    async fn test_executor_respects_local_only() {
        let ui = MockUI::new();
        let executor = TaskExecutor::new()
            .with_mesh_strategy(Arc::new(MockStrategy {
                name: "mesh",
                available: true,
                result: TaskResult::success("mesh done"),
            }))
            .with_local_strategy(Arc::new(MockStrategy {
                name: "local",
                available: true,
                result: TaskResult::success("local done"),
            }));

        let config = TaskConfig {
            use_local_only: true,
            ..Default::default()
        };

        let result = executor.run_task("test", &config, &ui).await.unwrap();

        assert!(result.success);
        assert_eq!(result.message, "local done");
    }

    #[tokio::test]
    async fn test_executor_mesh_only_fails_when_unavailable() {
        let ui = MockUI::new();
        let executor = TaskExecutor::new()
            .with_mesh_strategy(Arc::new(MockStrategy {
                name: "mesh",
                available: false,
                result: TaskResult::success("mesh done"),
            }))
            .with_local_strategy(Arc::new(MockStrategy {
                name: "local",
                available: true,
                result: TaskResult::success("local done"),
            }));

        let config = TaskConfig {
            mesh_only: true,
            ..Default::default()
        };

        let result = executor.run_task("test", &config, &ui).await.unwrap();

        assert!(!result.success);
        assert!(result.message.contains("No mesh agents available"));
    }
}
