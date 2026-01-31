//! Arkavo Session Management
//!
//! This crate handles communication sessions for the Arkavo agentic CLI tool.
//!
//! # Modules
//!
//! - `chat_session`: Manages chat sessions with LLM providers
//! - `websocket`: WebSocket connection handling for real-time communication
//! - `http`: HTTP session management and client handling
//! - `session_persistence`: Persistent storage for session state using SQLite
//! - `push_notifications`: Push notification delivery and management
//!
//! # Architecture
//!
//! The session crate provides a unified interface for managing various types
//! of communication sessions, abstracting the underlying transport mechanisms
//! and providing persistent state management.

#![warn(unreachable_pub)]

// Re-export from arkavo-protocol until fully migrated
pub use arkavo_protocol::chat_session;
pub use arkavo_protocol::http;
pub use arkavo_protocol::push_notifications;
pub use arkavo_protocol::session_persistence;
pub use arkavo_protocol::websocket;

// Re-export commonly used types
pub use arkavo_protocol::types::ChatSession;
pub use http::HttpTransport;
pub use websocket::WebSocketTransport;

use thiserror::Error;

/// Errors that can occur in session management
#[derive(Error, Debug)]
pub enum SessionError {
    /// Session not found
    #[error("session not found: {0}")]
    NotFound(String),

    /// Session expired
    #[error("session expired: {0}")]
    Expired(String),

    /// Connection error
    #[error("connection error: {0}")]
    Connection(String),

    /// Serialization error
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Database error
    #[error("database error: {0}")]
    Database(String),

    /// Generic error
    #[error("session error: {0}")]
    Other(String),
}

/// Result type for session operations
pub type Result<T> = std::result::Result<T, SessionError>;

/// Session identifier type
pub type SessionId = uuid::Uuid;

/// Trait for session lifecycle management
pub trait Session: Send + Sync {
    /// Returns the unique session identifier
    fn id(&self) -> SessionId;

    /// Returns true if the session is active
    fn is_active(&self) -> bool;

    /// Closes the session
    fn close(&mut self) -> Result<()>;
}
