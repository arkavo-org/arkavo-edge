//! Learning module for adaptive agent/model selection
//!
//! Implements Thompson Sampling for exploration-exploitation trade-off
//! with multi-layered feedback (immediate + retrospective credit assignment).
//!
//! # Overview
//!
//! The learning module maintains utility priors for each agent/model and
//! uses Thompson Sampling to select the best option while exploring
//! uncertain alternatives.
//!
//! # Key Components
//!
//! - **BetaPrior**: Beta distribution for modeling success probability
//! - **AgentUtility**: Per-agent utility tracking with category-specific priors
//! - **LearningModule**: Main orchestrator for Thompson Sampling
//! - **LearningConfig**: Configuration for learning parameters
//!
//! # Credit Assignment
//!
//! The module implements two-phase credit assignment:
//!
//! 1. **Immediate Update**: Called after each burst completes. Updates the
//!    agent's success/failure counts directly.
//!
//! 2. **Retrospective Update**: Called after a complete task finishes.
//!    Distributes reward backward through the contribution chain with
//!    discount factor γ=0.95 (later agents get more credit).
//!
//! # Cold Start Handling
//!
//! New agents start with Beta(2, 1) prior - optimistic but uncertain.
//! This encourages exploration while being somewhat favorable to new agents.
//! Agents graduate from probation after 50 successful observations.

mod agent_utility;
mod config;
pub mod coordination;
pub mod decision_trace;
mod feedback_types;
pub mod human_teaching;
mod module;
pub mod normalization;
mod persistence;
mod planner_score;
mod store;
pub mod tool_patterns;
pub mod uncertainty;
pub mod velocity;

pub use agent_utility::{AgentUtility, BetaPrior};
pub use config::LearningConfig;
pub use coordination::{CoordinationMetrics, MetricsCollector, MovingAverage};
pub use decision_trace::{DecisionTrace, SelectionReason};
pub use feedback_types::{AgentContribution, BurstFeedback, FinalTaskReport, QualityMetrics};
pub use human_teaching::TeachingIntent;
pub use module::{AgentUtilityStats, LearningModule};
pub use persistence::{
    Episode, EpisodeOutcome, Lesson, LessonPattern, LessonScope, LessonSource, MemoryTier,
    Observation,
};
pub use planner_score::{PlannerScore, SubtaskOutcome};
pub use store::{LearningStore, StoreError};
pub use tool_patterns::{ToolCallFormat, sanitize_args};
pub use uncertainty::UncertaintyEstimate;
pub use velocity::{LearningVelocity, VelocityTracker};
