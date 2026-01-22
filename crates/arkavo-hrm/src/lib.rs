//! HRM-Style Orchestration for Arkavo Edge
//!
//! This crate implements Hierarchical Reasoning Model (HRM) orchestration
//! for managing complex, multi-step AI agent tasks with bounded execution
//! contracts, state persistence, and verification.
//!
//! # Key Components
//!
//! - **Conductor**: Strategic task decomposition and GlobalTaskState management
//! - **BurstContract**: Bounded execution with budget enforcement
//! - **TaskStore**: Persistence layer for crash recovery
//!
//! # Features
//!
//! - State persistence for crash recovery
//! - Context handover between bursts via ContextStrategy
//! - Loop detection to prevent strategic thrashing
//! - Continuation hints for "almost done" scenarios
//!
//! # Example
//!
//! ```ignore
//! use arkavo_hrm::{Conductor, GlobalTaskState, TaskBudget, InMemoryTaskStore};
//!
//! let store = InMemoryTaskStore::new();
//! let conductor = Conductor::new(store);
//!
//! let budget = TaskBudget::default();
//! let task = conductor.create_task("Refactor authentication module", budget).await?;
//! ```

pub mod burst;
pub mod conductor;
pub mod error;
pub mod schemas;
pub mod store;
pub mod tools;

// Re-export main types
pub use burst::{BurstContract, BurstResult, BurstState, ContextStrategy, ContinuationHint};
pub use conductor::Conductor;
pub use error::{Error, Result};
pub use schemas::{
    BurstFeedback, FeedbackIssue, GlobalTaskState, ModelPattern, Priority, SubTask, SubTaskBudget,
    SubTaskResult, TaskBudget, TaskStatus, VerificationStatus,
};
pub use store::{InMemoryTaskStore, TaskStore};
