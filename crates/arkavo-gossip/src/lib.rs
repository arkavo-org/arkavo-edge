//! Gossip protocol for decentralized patch propagation
//!
//! This crate implements a gossip-based protocol for propagating
//! patchlets across a network of agents with quorum consensus.

mod consensus;
mod error;
mod message;

pub use consensus::{
    ConsensusState, ConsensusStatus, DEFAULT_QUORUM_THRESHOLD, DEFAULT_VOTE_TIMEOUT, QuorumConfig,
};
pub use error::{GossipError, GossipResult};
pub use message::{
    AntiEntropyDigest, GossipMessage, PatchAnnouncement, PatchDelivery, PatchDigestEntry,
    PatchRequest, PatchStatus, PatchVote,
};
