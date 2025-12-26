//! Gossip protocol for decentralized patch propagation
//!
//! This crate implements a gossip-based protocol for propagating
//! patchlets across a network of agents with quorum consensus.
//!
//! # Features
//!
//! - **Epidemic Gossip**: Efficient message propagation via fanout
//! - **Quorum Consensus**: 2/3 default threshold for patch approval
//! - **Zero-Trust Verification**: Ed25519 signature verification for all messages
//! - **Anti-Entropy**: Periodic synchronization to ensure consistency

mod consensus;
mod error;
pub mod learning_message;
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
pub use message::{
    AntiEntropyDigest, GossipMessage, PatchAnnouncement, PatchDelivery, PatchDigestEntry,
    PatchRequest, PatchStatus, PatchVote,
};
pub use protocol::{DEFAULT_ANTI_ENTROPY_INTERVAL, DEFAULT_FANOUT, GossipConfig, GossipProtocol};
pub use verification::{
    KeyRegistry, PatchVerifier, compute_content_hash, sign_announcement, sign_vote,
};
