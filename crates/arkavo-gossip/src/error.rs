//! Error types for gossip protocol operations

use thiserror::Error;
use uuid::Uuid;

/// Errors that can occur during gossip operations
#[derive(Debug, Error)]
pub enum GossipError {
    /// Message verification failed
    #[error("Verification failed: {0}")]
    Verification(String),

    /// Unknown originator for message
    #[error("Unknown originator: {0}")]
    UnknownOriginator(String),

    /// Signature verification failed
    #[error("Signature verification failed")]
    SignatureVerification(#[from] arkavo_crypto::CryptoError),

    /// Quorum not reached for consensus
    #[error("Quorum not reached: {votes}/{required} votes")]
    QuorumNotReached { votes: usize, required: usize },

    /// Patch not found
    #[error("Patch not found: {0}")]
    PatchNotFound(Uuid),

    /// Lesson not found
    #[error("Lesson not found: {0}")]
    LessonNotFound(Uuid),

    /// Consensus timed out
    #[error("Consensus timed out for patch {0}")]
    ConsensusTimeout(Uuid),

    /// Duplicate message received
    #[error("Duplicate message: {0}")]
    Duplicate(Uuid),

    /// Protocol error
    #[error("Protocol error: {0}")]
    Protocol(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Peer rate limited
    #[error("Rate limited: peer {0} exceeded message limit")]
    RateLimited(String),

    /// A different key is already bound to this agent id
    #[error("a different key is already bound to {0}")]
    KeyConflict(String),

    /// The agent id belongs to a workload key, and the key offered for it
    /// comes without that workload key's statement
    #[error("{agent_id} is the agent {did}; a key for it needs that agent's key statement")]
    Unvouched { agent_id: String, did: String },

    /// The agent id belongs to another workload key than the one vouching
    #[error("{agent_id} is the agent {pinned}, not {presented}; a new agent needs a new name")]
    IdentityConflict {
        agent_id: String,
        pinned: String,
        presented: String,
    },

    /// A key statement that does not hold
    #[error(transparent)]
    Statement(#[from] arkavo_key_statement::StatementError),

    /// Message rejected because it originated from a different swarm
    #[error("swarm mismatch: message from '{0}' rejected")]
    SwarmMismatch(String),
}

/// Result type alias for gossip operations
pub type GossipResult<T> = Result<T, GossipError>;
