//! # Arkavo Agent Lifecycle Management
//!
//! This crate provides comprehensive agent lifecycle management for the Arkavo platform.
//!
//! ## Module Overview
//!
//! - [`agent_config`]: Configuration management for agents, including settings validation
//!   and persistence.
//! - [`agent_registry`]: Central registry for tracking agent instances, their states,
//!   and metadata.
//! - [`discovery`]: Service discovery mechanisms for locating agents within the network,
//!   including mDNS and directory-based discovery.
//! - [`registration`]: Agent registration and authentication with the Arkavo control plane.
//!
//! ## Architecture
//!
//! The agent lifecycle follows these stages:
//!
//! 1. **Configuration**: Load and validate agent configuration
//! 2. **Registration**: Authenticate and register with the control plane
//! 3. **Discovery**: Locate peers and services in the network
//! 4. **Operation**: Execute agent tasks with full observability
//! 5. **Deregistration**: Graceful shutdown and cleanup

#![warn(unreachable_pub)]

// Re-export from arkavo-protocol until fully migrated
pub use arkavo_protocol::agent_config;
pub use arkavo_protocol::agent_registry;
pub use arkavo_protocol::discovery;
pub use arkavo_protocol::registration;
