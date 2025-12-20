//! Error types for SAT operations

use thiserror::Error;

/// Errors that can occur during SAT operations
#[derive(Debug, Error)]
pub enum SatError {
    /// CNF extraction failed
    #[error("CNF extraction failed: {0}")]
    CnfExtraction(String),

    /// SAT solver error
    #[error("SAT solver error: {0}")]
    Solver(String),

    /// No solution found (UNSAT)
    #[error("No satisfying assignment exists (UNSAT)")]
    Unsatisfiable,

    /// Invalid graph structure
    #[error("Invalid graph structure: {0}")]
    InvalidGraph(String),

    /// Boundary probing failed
    #[error("Boundary probing failed: {0}")]
    BoundaryProbe(String),
}

/// Result type alias for SAT operations
pub type SatResult<T> = Result<T, SatError>;
