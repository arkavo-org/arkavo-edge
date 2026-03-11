//! Autoresearch: autonomous parameter tuning subsystem
//!
//! Implements Karpathy's autoresearch pattern (iterate → evaluate → keep/revert)
//! as a runtime subsystem within the Conductor. Supports:
//! - Thompson Sampling for intelligent exploration
//! - BurstContract-compatible wall-time budgets
//! - Distributed coordination via gossip ExperimentMessage
//! - Weighted quality metric: quality_per_token × ln(1 + tok/s)

mod config;
mod evaluator;
mod thompson;

pub use config::{ExperimentConfig, ExperimentResult, SearchSpace};
pub use evaluator::evaluate_quality;
pub use thompson::{BetaPrior, ThompsonSampler};
