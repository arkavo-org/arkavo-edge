use std::path::PathBuf;

/// Configuration for task execution
#[derive(Debug, Clone, Default)]
pub struct TaskConfig {
    /// Skip user approval before executing plan
    pub auto_approve: bool,
    /// Push changes to remote after commit
    pub push: bool,
    /// Run validation (fmt, clippy) before commit
    pub validate: bool,
    /// Custom commit message (auto-generated if None)
    pub commit_message: Option<String>,
    /// Only use local models, skip mesh
    pub use_local_only: bool,
    /// Only use mesh agents, fail if unavailable
    pub mesh_only: bool,
    /// Target specific agent by ID
    pub target_agent_id: Option<String>,
}

/// Result of task execution
#[derive(Debug, Clone)]
pub struct TaskResult {
    /// Whether the task completed successfully
    pub success: bool,
    /// Human-readable result message
    pub message: String,
    /// Git commit OID if changes were committed
    pub commit_oid: Option<String>,
    /// Number of files changed
    pub changes_count: usize,
}

impl TaskResult {
    pub fn success(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: message.into(),
            commit_oid: None,
            changes_count: 0,
        }
    }

    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
            commit_oid: None,
            changes_count: 0,
        }
    }

    pub fn with_commit(mut self, oid: String) -> Self {
        self.commit_oid = Some(oid);
        self
    }

    pub fn with_changes(mut self, count: usize) -> Self {
        self.changes_count = count;
        self
    }
}

/// Model capability tier based on parameter count
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCapability {
    /// Less than 2B parameters - info gathering, simple tasks
    Small,
    /// 2-7B parameters - planning, tool use, reasoning
    Medium,
    /// Greater than 7B parameters - complex planning, multi-step
    Large,
}

/// Information about an available model
#[derive(Debug, Clone)]
pub struct ModelInfo {
    /// Model name for display
    pub name: String,
    /// Provider name (e.g., "gemini", "openai", "local")
    pub provider: String,
    /// Model identifier for API calls
    pub model_id: String,
    /// Capability tier
    pub capability: ModelCapability,
    /// Size in GB (for local models)
    pub size_gb: Option<f64>,
    /// Path to model file (for local GGUF models)
    pub path: Option<PathBuf>,
}

impl ModelInfo {
    pub fn cloud(
        name: impl Into<String>,
        provider: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            provider: provider.into(),
            model_id: model_id.into(),
            capability: ModelCapability::Large,
            size_gb: None,
            path: None,
        }
    }

    pub fn local(
        name: impl Into<String>,
        path: PathBuf,
        size_gb: f64,
        capability: ModelCapability,
    ) -> Self {
        let name = name.into();
        Self {
            model_id: path.to_string_lossy().to_string(),
            name,
            provider: "local".to_string(),
            capability,
            size_gb: Some(size_gb),
            path: Some(path),
        }
    }
}

/// Models selected for task execution
#[derive(Debug, Clone)]
pub struct SelectedModels {
    /// Model for information gathering (typically local)
    pub gather_model: ModelInfo,
    /// Model for planning (typically cloud)
    pub planning_model: ModelInfo,
    /// Model for verification (typically local)
    pub verify_model: ModelInfo,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_result_builders() {
        let result = TaskResult::success("Done")
            .with_commit("abc123".to_string())
            .with_changes(5);

        assert!(result.success);
        assert_eq!(result.commit_oid, Some("abc123".to_string()));
        assert_eq!(result.changes_count, 5);
    }

    #[test]
    fn test_model_info_cloud() {
        let model = ModelInfo::cloud("Gemini Flash", "gemini", "gemini-2.0-flash");
        assert_eq!(model.provider, "gemini");
        assert!(model.path.is_none());
    }

    #[test]
    fn test_model_info_local() {
        let model = ModelInfo::local(
            "Qwen",
            PathBuf::from("/path/to/model.gguf"),
            2.5,
            ModelCapability::Medium,
        );
        assert_eq!(model.provider, "local");
        assert!(model.path.is_some());
        assert_eq!(model.size_gb, Some(2.5));
    }
}
