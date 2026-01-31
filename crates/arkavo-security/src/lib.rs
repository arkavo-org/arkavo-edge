//! # Arkavo Security
//!
//! This crate consolidates all security-related functionality for the Arkavo agentic CLI.
//!
//! ## Modules
//!
//! - **`auth`** - Core authentication primitives and utilities
//! - **`oauth2`** - OAuth2 client implementation for third-party integrations
//! - **`security`** - General security utilities, token management, and validation
//! - **`security_fixes`** - Patches and mitigations for known security vulnerabilities
//! - **`rate_limit`** - Rate limiting implementation for API protection
//! - **`data_classification`** - Data classification and PII detection for DLP
//!
//! ## Features
//!
//! - JWT token generation and validation
//! - OAuth2 flow handling
//! - Configurable rate limiting with token bucket algorithm
//! - Data classification for sensitive information detection
//! - Security vulnerability patches and mitigations
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

// Re-export from arkavo-protocol until fully migrated
pub use arkavo_protocol::auth;
pub use arkavo_protocol::data_classification;
pub use arkavo_protocol::oauth2;
pub use arkavo_protocol::rate_limit;
pub use arkavo_protocol::security;
pub use arkavo_protocol::security_fixes;

/// Re-export common types for convenience.
pub mod prelude {
    pub use arkavo_protocol::security::{SecurityConfig, TlsSettings};
}

/// Crate version information.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
