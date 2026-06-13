//! Local-model evaluation pipeline: resolves an eval contract, gates on
//! preconditions, runs the model, and produces a typed regression verdict.

pub mod contract;
pub mod digest;
pub mod gate;
pub mod plan;
pub mod status;
