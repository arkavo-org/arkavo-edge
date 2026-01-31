//! Task management system for Arkavo.
//!
//! This crate provides the core task management infrastructure for the Arkavo agentic CLI,
//! handling task execution, planning, and persistent storage. It integrates with the HRM
//! (Human Resource Management) orchestration system to provide bounded, observable AI agent
//! execution.
//!
//! ## Architecture
//!
//! The crate is organized into three main modules:
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
//! ## Integration
//!
//! This crate integrates with:
//! - `arkavo-hrm` for orchestration and resource management
//! - `arkavo-events` for task lifecycle event publishing (via HRM)
//! - `arkavo-memory` for context-aware task execution (via HRM)

#![warn(unreachable_pub)]

// Re-export from arkavo-protocol until fully migrated
pub use arkavo_protocol::task_executor;
pub use arkavo_protocol::task_planner;
pub use arkavo_protocol::task_store;
