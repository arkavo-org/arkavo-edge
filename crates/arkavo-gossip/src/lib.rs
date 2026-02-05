//! Gossip protocol for decentralized patch and lesson propagation
//!
//! This crate implements a gossip-based protocol for propagating
//! patchlets and lessons across a network of agents with quorum consensus.
//!
//! # Features
//!
//! - **Epidemic Gossip**: Efficient message propagation via fanout
//! - **Quorum Consensus**: 2/3 default threshold for patch/lesson approval
//! - **Zero-Trust Verification**: Ed25519 signature verification for all messages
//! - **Anti-Entropy**: Periodic synchronization to ensure consistency
//! - **Cross-Swarm Learning**: Lesson propagation and consensus

mod consensus;
pub mod context_message;
mod error;
pub mod learning_message;
mod lesson_consensus;
mod lesson_handlers;
mod message;
mod protocol;
mod verification;

pub use consensus::{
    ConsensusState, ConsensusStatus, DEFAULT_QUORUM_THRESHOLD, DEFAULT_VOTE_TIMEOUT, QuorumConfig,
};
pub use error::{GossipError, GossipResult};
pub use learning_message::{
    LessonAnnouncement, LessonDelivery, LessonDigest, LessonDigestEntry, LessonRequest,
    LessonStatus, LessonVote, LocalEvidence,
};
pub use lesson_consensus::{LessonConsensusState, LessonConsensusStatus};
pub use message::{
    AntiEntropyDigest, GossipMessage, PatchAnnouncement, PatchDelivery, PatchDigestEntry,
    PatchRequest, PatchStatus, PatchVote,
};
pub use protocol::{
    DEFAULT_ANTI_ENTROPY_INTERVAL, DEFAULT_FANOUT, GossipConfig, GossipProtocol, GossipStats,
};
pub use verification::{
    KeyRegistry, PatchVerifier, compute_content_hash, sign_announcement, sign_lesson_announcement,
    sign_lesson_vote, sign_vote,
};
