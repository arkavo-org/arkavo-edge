//! Iroh P2P data plane for TDF blob transport.
//!
//! This crate implements the `BlobTransport` trait from `arkavo-tdf` using
//! Iroh's content-addressed blob storage and P2P transfer protocol.
//!
//! # Architecture
//!
//! ```text
//! Control Plane (existing): mDNS discovery, WebSocket/MCP for signaling
//! Data Plane (this crate): Embedded Iroh node for blob transport
//! ```
//!
//! # Important Design Note
//!
//! This crate handles **raw byte transport only**. It does NOT perform encryption.
//! Encryption should be composed at the application layer by combining
//! `TdfEncryptor` + `BlobTransport`.
//!
//! # Example
//!
//! ```ignore
//! use arkavo_tdf_iroh::{IrohNode, IrohTransport};
//! use arkavo_tdf::BlobTransport;
//!
//! // Create node and transport
//! let node = IrohNode::memory().await?;
//! let transport = IrohTransport::new(node);
//!
//! // Stage some data
//! let ticket = transport.stage(b"Hello, World!").await?;
//!
//! // Share ticket via control plane (MCP/WebSocket)
//! // Peer can then fetch:
//! let data = transport.fetch(&ticket).await?;
//! ```

mod error;
mod node;
mod ticket;
mod transport;

pub use error::IrohError;
pub use node::{IrohNode, IrohNodeConfig};
pub use ticket::{IrohTicket, SerializableTicket};
pub use transport::IrohTransport;
