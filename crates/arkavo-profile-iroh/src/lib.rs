//! Creator profile storage and sync service using Iroh P2P.
//!
//! This crate provides the backend service for iroh.arkavo.net,
//! enabling creator profile storage and synchronization between
//! ArkavoCreator (macOS) and Arkavo (iOS) apps.

pub mod api;
pub mod auth;
pub mod config;
pub mod profile;

pub use config::Config;
pub use profile::{CreatorProfile, ProfileStore};
