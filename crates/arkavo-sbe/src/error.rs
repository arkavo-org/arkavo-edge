//! Error types for Symbolic Boundary Evolution

use thiserror::Error;
use uuid::Uuid;

use crate::layer::GraphLayer;

/// Errors that can occur in SBE operations
#[derive(Debug, Error)]
pub enum SbeError {
    /// Attempted to modify a node in a protected layer
    #[error(
        "Layer violation: attempted to modify {actual} layer node {node_id} from {attempted} layer"
    )]
    LayerViolation {
        attempted: GraphLayer,
        actual: GraphLayer,
        node_id: u16,
    },

    /// Patchlet affects too many nodes
    #[error("Scope exceeded: patchlet affects {affected} nodes, max allowed is {max}")]
    ScopeExceeded { affected: usize, max: usize },

    /// Too many patchlets accumulated, requires recompilation
    #[error("Recompile required: {count} patchlets accumulated, max is {max}")]
    RecompileRequired { count: usize, max: usize },

    /// Node not found in graph
    #[error("Node {0} not found in graph")]
    NodeNotFound(u16),

    /// Invariant contract was violated
    #[error("Invariant violation: contract {contract_id} violated - {description}")]
    InvariantViolation {
        contract_id: Uuid,
        description: String,
    },

    /// Signature verification failed
    #[error("Signature verification failed: {0}")]
    SignatureVerification(String),

    /// Invalid contract format
    #[error("Invalid contract: {0}")]
    InvalidContract(String),

    /// Rollback handle expired
    #[error("Rollback expired: handle for patchlet {0} is no longer valid")]
    RollbackExpired(Uuid),

    /// Graph evaluation error
    #[error("Evaluation error: {0}")]
    Evaluation(#[from] torg_core::EvalError),

    /// Graph build error
    #[error("Build error: {0}")]
    Build(#[from] torg_core::BuildError),

    /// Crypto error from arkavo-crypto
    #[error("Crypto error: {0}")]
    Crypto(#[from] arkavo_crypto::CryptoError),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Quorum not reached for policy update
    #[error("Quorum not reached: {votes}/{required} votes received")]
    QuorumNotReached { votes: usize, required: usize },

    /// Storage error
    #[error("Storage error: {0}")]
    Storage(String),
}

/// Result type alias for SBE operations
pub type SbeResult<T> = Result<T, SbeError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = SbeError::LayerViolation {
            attempted: GraphLayer::Adaptive,
            actual: GraphLayer::Invariant,
            node_id: 5,
        };
        let msg = err.to_string();
        assert!(msg.contains("Invariant"));
        assert!(msg.contains("Adaptive"));
        assert!(msg.contains("5"));
    }

    #[test]
    fn test_scope_exceeded_display() {
        let err = SbeError::ScopeExceeded {
            affected: 15,
            max: 10,
        };
        assert!(err.to_string().contains("15"));
        assert!(err.to_string().contains("10"));
    }
}
