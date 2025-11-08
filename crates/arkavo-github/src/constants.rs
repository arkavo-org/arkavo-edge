use std::path::PathBuf;

pub const DEFAULT_ARKAVO_DIR: &str = ".arkavo";
pub const ORCHESTRATOR_STATE_DB: &str = "orchestrator-org-state.db";
pub const ORCHESTRATOR_TASKS_DB: &str = "orchestrator-tasks.db";

pub fn default_state_db_path() -> PathBuf {
    PathBuf::from(DEFAULT_ARKAVO_DIR).join(ORCHESTRATOR_STATE_DB)
}

pub fn default_tasks_db_path() -> PathBuf {
    PathBuf::from(DEFAULT_ARKAVO_DIR).join(ORCHESTRATOR_TASKS_DB)
}
