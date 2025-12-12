//! Local OpenTDF stack orchestration for Arkavo Edge.
//!
//! This crate provides functionality to manage a local OpenTDF platform stack
//! using container runtimes (Apple container CLI, Docker, or Podman).
//!
//! # Features
//!
//! - Auto-detect container runtime (prefers Apple container CLI on macOS)
//! - Start/stop/status management for the full OpenTDF stack
//! - Health checks for Keycloak and OpenTDF services
//! - Endpoint discovery for integration with arkavo-tdf
//!
//! # Example
//!
//! ```no_run
//! use arkavo_opentdf_local::{OpenTdfStack, StackStatus};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Check if stack is already running
//!     if let Some(stack) = OpenTdfStack::detect().await {
//!         let endpoints = stack.get_endpoints().await?;
//!         println!("KAS URL: {}", endpoints.kas_url);
//!         return Ok(());
//!     }
//!
//!     // Start a new stack
//!     let stack = OpenTdfStack::new()?;
//!     stack.start().await?;
//!
//!     let endpoints = stack.get_endpoints().await?;
//!     println!("OpenTDF ready at: {}", endpoints.kas_url);
//!
//!     Ok(())
//! }
//! ```

mod config;
mod error;
mod health;
mod runtime;
mod stack;

pub use config::{OpenTdfEndpoints, StackConfig};
pub use error::{OpenTdfLocalError, Result};
pub use runtime::{ContainerRuntime, detect_runtime};
pub use stack::{ContainerInfo, OpenTdfStack, StackStatus};
