//! # Arkavo Security
//!
//! This crate consolidates all security-related functionality for the Arkavo agentic CLI.
//!
//! ## Modules
//!
//! - **`auth`** - Core authentication primitives and utilities
//! - **`oauth2`** - OAuth2 client implementation for third-party integrations
//! - **`security`** - General security utilities, token management, and validation
//! - **`rate_limit`** - Rate limiting implementation for API protection
//! - **`data_classification`** - Data classification and PII detection for DLP
//!
//! ## Features
//!
//! - JWT token generation and validation
//! - OAuth2 flow handling
//! - Configurable rate limiting with token bucket algorithm
//! - Data classification for sensitive information detection
//! - Centralized input validation via `arkavo-validation`
//!
//! ## Security
//!
//! This crate implements security best practices including:
//! - No hardcoded secrets or API keys
//! - Secure defaults for all configurations
//! - Constant-time comparison for sensitive operations
//! - Proper entropy sources for cryptographic operations

#![warn(missing_docs)]
#![warn(unreachable_pub)]

pub mod auth;
pub mod data_classification;
pub mod error;
pub mod oauth2;
pub mod rate_limit;
pub mod security;

/// Re-export common types for convenience.
pub mod prelude {
    pub use crate::auth::{AuthBackend, JwtAuthBackend, SessionAuth};
    pub use crate::error::{Result, SecurityError};
    pub use crate::security::{SecurityConfig, TlsSettings};
}

/// Crate version information.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
