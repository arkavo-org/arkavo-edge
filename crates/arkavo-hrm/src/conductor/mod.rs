//! Conductor module for strategic task orchestration
//!
//! The Conductor is responsible for:
//! - Task decomposition into subtasks
//! - State management via TaskStore
//! - Loop detection to prevent thrashing
//! - Budget enforcement

mod loop_detector;
mod orchestrator;

pub use loop_detector::LoopDetector;
pub use orchestrator::Conductor;
