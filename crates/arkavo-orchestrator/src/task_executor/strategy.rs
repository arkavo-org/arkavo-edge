use async_trait::async_trait;

use super::config::{TaskConfig, TaskResult};
use super::ui::TaskUI;
use crate::error::Result;

/// Strategy for executing tasks
///
/// Implementations handle different execution contexts:
/// - Local: Uses local/cloud models directly
/// - Mesh: Delegates to mesh agents via A2A protocol
#[async_trait]
pub trait TaskStrategy: Send + Sync {
    /// Check if this strategy is available for execution
    async fn is_available(&self) -> bool;

    /// Execute the task using this strategy
    async fn execute(
        &self,
        task: &str,
        config: &TaskConfig,
        ui: &dyn TaskUI,
    ) -> Result<TaskResult>;

    /// Get strategy name for logging/display
    fn name(&self) -> &str;
}
