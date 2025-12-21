//! Burst contract module for bounded execution
//!
//! A BurstContract defines the boundaries of a single unit of work:
//! maximum steps, wall time, tokens, and cost. This prevents runaway
//! execution and enables predictable resource consumption.

mod contract;
mod result;

pub use contract::{BurstContract, BurstState, ContextStrategy};
pub use result::{BurstResult, ContinuationHint};
