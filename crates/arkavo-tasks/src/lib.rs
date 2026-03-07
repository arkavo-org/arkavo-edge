//! Task management system for Arkavo.
//!
//! This crate provides the core task management infrastructure for the Arkavo agentic CLI,
//! handling task execution, planning, and persistent storage. It integrates with the HRM
//! (Human Resource Management) orchestration system to provide bounded, observable AI agent
//! execution.
//!
//! ## Architecture
//!
//! The crate is organized into four main modules:
//!
//! - **`agent_registry`**: Tracks available agents and their capabilities for task routing.
//!
//! - **`error`**: Error types for task operations, including the main [`TaskError`] enum.
//!
//! - **`task_executor`**: Handles the actual execution of tasks, managing the lifecycle
//!   from submission through completion or failure. Provides async execution with
//!   cancellation support and resource limits.
//!
//! - **`task_planner`**: Plans and schedules tasks based on dependencies, priorities,
//!   and available resources. Includes DAG-based dependency resolution and conflict
//!   detection.
//!
//! - **`task_store`**: Persistent storage for tasks using SQLite via sqlx. Supports
//!   querying task history, resuming interrupted tasks, and audit logging.
//!
//! - **`types`**: Core types for task management including [`Task`], [`TaskStatus`],
//!   [`Message`], [`AgentCard`], and more.
//!
//! ## Integration
//!
//! This crate integrates with:
//! - `arkavo-hrm` for orchestration and resource management
//! - `arkavo-events` for task lifecycle event publishing (via HRM)
//! - `arkavo-memory` for context-aware task execution (via HRM)

#![warn(unreachable_pub)]

pub mod agent_registry;
pub mod error;
pub mod intent_analyzer;
pub mod task_executor;
pub mod task_planner;
pub mod task_store;
pub mod types;

// Re-export main types
pub use agent_registry::{AgentInfo, AgentRegistry};
pub use error::{Result, TaskError};
pub use intent_analyzer::{Entity, IntentAnalysis, IntentAnalyzer, RuleBasedAnalyzer, SubTaskSpec};
pub use task_executor::{MetricsCollector, TaskEvent, TaskExecutor, TaskExecutorConfig};
pub use task_planner::{SubTask, SubTaskStatus, TaskPlan, TaskPlanError, TaskPlanner};
pub use task_store::{SqliteTaskStore, TaskStore};
pub use types::{
    AgentCapabilities, AgentCard, AgentExtension, AgentProvider, AgentSkill, AiCapability,
    DeviceCapabilities, DevicePlatform, Message, MessagePart, SecurityRequirement, SecurityScheme,
    SecuritySchemeType, SensorType, Task, TaskErrorInfo, TaskOffer, TaskProgress, TaskStatus,
};
