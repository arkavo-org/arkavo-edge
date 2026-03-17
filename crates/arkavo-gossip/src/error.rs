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

    /// Message from a different swarm
    #[error("swarm mismatch: message from '{0}' rejected")]
    SwarmMismatch(String),
}

/// Result type alias for gossip operations
pub type GossipResult<T> = Result<T, GossipError>;
