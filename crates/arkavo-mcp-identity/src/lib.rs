//! MCP-I (Model Context Protocol - Identity) Level 1 implementation.
//!
//! Extends MCP with cryptographic identity and signed proofs per the
//! MCP-I specification at <https://modelcontextprotocol-identity.io>.
//!
//! Level 1 provides:
//! - DID:key identity auto-generation from device keypair
//! - Detached JWS (EdDSA/Ed25519) signed proofs on tool responses
//! - `_mcpi` protocol tool for identity disclosure and session state
//! - In-memory peer reputation tracking

pub mod jws;
pub mod protocol_tool;
pub mod session;

pub use protocol_tool::register_identity_tool;
pub use session::McpIdentitySession;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum IdentityError {
    #[error("JWS error: {0}")]
    Jws(String),

    #[error("Session error: {0}")]
    Session(String),

    #[error("Crypto error: {0}")]
    Crypto(#[from] arkavo_crypto::CryptoError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Device identity error: {0}")]
    DeviceIdentity(String),
}
