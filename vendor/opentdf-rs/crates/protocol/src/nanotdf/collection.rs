//! NanoTDF Collection/Dataset Protocol Types
//!
//! This module defines the protocol types for NanoTDF Collections, enabling
//! multiple encrypted payloads to share a single NanoTDF header (manifest) and DEK.
//!
//! # Overview
//!
//! A NanoTDF Collection consists of:
//! - A single manifest (NanoTDF header) containing KAS info, policy, and ephemeral public key
//! - Multiple collection items, each with a unique counter-based IV
//!
//! This supports streaming use cases like NTDF-RTMP where the manifest is sent once
//! and payloads are encrypted per-frame with minimal overhead.
//!
//! # Wire Formats
//!
//! ## Container framing (for FLV/RTMP where container provides length):
//! ```text
//! [IV (3B, counter-based)][Ciphertext][Auth Tag]
//! ```
//!
//! ## NanoTDF payload framing (spec-compliant with length prefix):
//! ```text
//! [Length (3B, big-endian)][IV (3B)][Ciphertext][Auth Tag]
//! ```
//!
//! # IV Management
//!
//! - IV 0x000000 is reserved for encrypted policy (never used for items)
//! - IV range: 1 to 0x00FFFFFF (16,777,215 items max per DEK)
//! - GCM nonce expansion: `[9 zero bytes][3-byte IV counter]` (JS SDK interop)

use crate::binary::{read_u24_be, write_u24_be};
use std::io::{self, Read, Write};

/// Maximum IV value (2^24 - 1 = 16,777,215)
pub const MAX_IV: u32 = 0x00FF_FFFF;

/// Reserved IV for policy encryption (must not be used for collection items)
pub const RESERVED_POLICY_IV: u32 = 0;

/// Default rotation threshold (conservative: 2^23 = 8,388,608)
/// This provides an early warning signal before IV exhaustion
pub const DEFAULT_ROTATION_THRESHOLD: u32 = 0x0080_0000;

/// Collection item containing an encrypted payload with its IV
///
/// Each item uses a unique counter-based IV derived from the collection's
/// IV counter. The ciphertext_and_tag contains the AES-GCM encrypted data
/// with the authentication tag appended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionItem {
    /// 24-bit IV counter value (1 to MAX_IV)
    /// This is used to reconstruct the 12-byte GCM nonce: [9 zeros][3-byte IV]
    pub iv: u32,

    /// Ciphertext with authentication tag appended
    /// Format: [encrypted_data][auth_tag]
    /// Tag size depends on the symmetric cipher config in the collection manifest
    pub ciphertext_and_tag: Vec<u8>,
}

impl CollectionItem {
    /// Create a new collection item
    pub fn new(iv: u32, ciphertext_and_tag: Vec<u8>) -> Self {
        Self {
            iv,
            ciphertext_and_tag,
        }
    }

    /// Container framing: [IV (3B)][ciphertext][tag]
    ///
    /// Use this format when the container (FLV/RTMP) provides the length.
    /// This is the minimal wire format for streaming scenarios.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(3 + self.ciphertext_and_tag.len());
        // Write IV as 3 bytes big-endian
        buf.push(((self.iv >> 16) & 0xFF) as u8);
        buf.push(((self.iv >> 8) & 0xFF) as u8);
        buf.push((self.iv & 0xFF) as u8);
        buf.extend_from_slice(&self.ciphertext_and_tag);
        buf
    }

    /// NanoTDF payload framing: [length (3B)][IV (3B)][ciphertext][tag]
    ///
    /// This is spec-compliant format with a 3-byte length prefix.
    /// Use this when you need a self-describing payload format.
    pub fn to_nanotdf_payload_bytes(&self) -> Vec<u8> {
        let payload_len = 3 + self.ciphertext_and_tag.len();
        let mut buf = Vec::with_capacity(3 + payload_len);

        // Write length (3 bytes big-endian) - includes IV + ciphertext + tag
        buf.push(((payload_len >> 16) & 0xFF) as u8);
        buf.push(((payload_len >> 8) & 0xFF) as u8);
        buf.push((payload_len & 0xFF) as u8);

        // Write IV as 3 bytes big-endian
        buf.push(((self.iv >> 16) & 0xFF) as u8);
        buf.push(((self.iv >> 8) & 0xFF) as u8);
        buf.push((self.iv & 0xFF) as u8);

        buf.extend_from_slice(&self.ciphertext_and_tag);
        buf
    }

    /// Parse from container framing: [IV (3B)][ciphertext][tag]
    ///
    /// The caller must know the total length from the container.
    pub fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < 3 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Collection item too small: need at least 3 bytes for IV",
            ));
        }

        // Read IV from first 3 bytes (big-endian)
        let iv = ((bytes[0] as u32) << 16) | ((bytes[1] as u32) << 8) | (bytes[2] as u32);

        // Validate IV is in valid range (not reserved, not over max)
        if iv == RESERVED_POLICY_IV {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Collection item has reserved IV 0 (used for encrypted policy)",
            ));
        }
        if iv > MAX_IV {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Collection item IV {} exceeds MAX_IV {}", iv, MAX_IV),
            ));
        }

        let ciphertext_and_tag = bytes[3..].to_vec();

        Ok(Self {
            iv,
            ciphertext_and_tag,
        })
    }

    /// Parse from NanoTDF payload framing: [length (3B)][IV (3B)][ciphertext][tag]
    ///
    /// Reads the length prefix and validates the payload size.
    pub fn from_nanotdf_payload_bytes(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < 6 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "NanoTDF payload too small: need at least 6 bytes (3 length + 3 IV)",
            ));
        }

        // Read length from first 3 bytes (big-endian)
        let length = ((bytes[0] as usize) << 16) | ((bytes[1] as usize) << 8) | (bytes[2] as usize);

        // Validate we have enough data
        if bytes.len() < 3 + length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "NanoTDF payload truncated: expected {} bytes, got {}",
                    3 + length,
                    bytes.len()
                ),
            ));
        }

        // Parse the payload portion (after length prefix)
        Self::from_bytes(&bytes[3..3 + length])
    }

    /// Read from a reader using container framing
    ///
    /// Reads exactly `total_len` bytes (IV + ciphertext + tag).
    pub fn read_from<R: Read>(reader: &mut R, total_len: usize) -> io::Result<Self> {
        if total_len < 3 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Collection item too small: need at least 3 bytes for IV",
            ));
        }

        let mut bytes = vec![0u8; total_len];
        reader.read_exact(&mut bytes)?;
        Self::from_bytes(&bytes)
    }

    /// Read from a reader using NanoTDF payload framing
    ///
    /// Reads the length prefix first, then the payload.
    pub fn read_nanotdf_payload_from<R: Read>(reader: &mut R) -> io::Result<Self> {
        // Read length (3 bytes big-endian)
        let length = read_u24_be(reader)? as usize;

        if length < 3 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "NanoTDF payload length too small: need at least 3 bytes for IV",
            ));
        }

        let mut payload = vec![0u8; length];
        reader.read_exact(&mut payload)?;
        Self::from_bytes(&payload)
    }

    /// Write to a writer using container framing
    pub fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        // Write IV as 3 bytes big-endian
        writer.write_all(&[
            ((self.iv >> 16) & 0xFF) as u8,
            ((self.iv >> 8) & 0xFF) as u8,
            (self.iv & 0xFF) as u8,
        ])?;
        writer.write_all(&self.ciphertext_and_tag)
    }

    /// Write to a writer using NanoTDF payload framing
    pub fn write_nanotdf_payload_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let payload_len = (3 + self.ciphertext_and_tag.len()) as u32;
        write_u24_be(writer, payload_len)?;
        self.write_to(writer)
    }

    /// Get the total size in container framing format
    pub fn container_size(&self) -> usize {
        3 + self.ciphertext_and_tag.len()
    }

    /// Get the total size in NanoTDF payload framing format
    pub fn nanotdf_payload_size(&self) -> usize {
        3 + 3 + self.ciphertext_and_tag.len()
    }

    /// Convert the IV counter to a 3-byte array
    pub fn iv_bytes(&self) -> [u8; 3] {
        [
            ((self.iv >> 16) & 0xFF) as u8,
            ((self.iv >> 8) & 0xFF) as u8,
            (self.iv & 0xFF) as u8,
        ]
    }

    /// Convert the IV to a 12-byte GCM nonce
    ///
    /// Per NanoTDF spec and JS SDK interop: [9 zero bytes][3-byte IV counter]
    pub fn to_gcm_nonce(&self) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        nonce[9] = ((self.iv >> 16) & 0xFF) as u8;
        nonce[10] = ((self.iv >> 8) & 0xFF) as u8;
        nonce[11] = (self.iv & 0xFF) as u8;
        nonce
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_collection_item_container_roundtrip() {
        let item = CollectionItem::new(1, vec![0x01, 0x02, 0x03, 0x04]);
        let bytes = item.to_bytes();

        // Should be 3 (IV) + 4 (data) = 7 bytes
        assert_eq!(bytes.len(), 7);

        let parsed = CollectionItem::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.iv, 1);
        assert_eq!(parsed.ciphertext_and_tag, vec![0x01, 0x02, 0x03, 0x04]);
    }

    #[test]
    fn test_collection_item_nanotdf_payload_roundtrip() {
        let item = CollectionItem::new(0x123456, vec![0xAA, 0xBB, 0xCC]);
        let bytes = item.to_nanotdf_payload_bytes();

        // Should be 3 (length) + 3 (IV) + 3 (data) = 9 bytes
        assert_eq!(bytes.len(), 9);

        let parsed = CollectionItem::from_nanotdf_payload_bytes(&bytes).unwrap();
        assert_eq!(parsed.iv, 0x123456);
        assert_eq!(parsed.ciphertext_and_tag, vec![0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn test_iv_to_gcm_nonce() {
        let item = CollectionItem::new(0x010203, vec![]);
        let nonce = item.to_gcm_nonce();

        // First 9 bytes should be zeros
        assert_eq!(&nonce[0..9], &[0u8; 9]);
        // Last 3 bytes should be the IV
        assert_eq!(&nonce[9..12], &[0x01, 0x02, 0x03]);
    }

    #[test]
    fn test_reserved_iv_rejected() {
        let bytes = [0x00, 0x00, 0x00, 0x01, 0x02]; // IV = 0 (reserved)
        let result = CollectionItem::from_bytes(&bytes);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("reserved"));
    }

    #[test]
    fn test_max_iv() {
        let item = CollectionItem::new(MAX_IV, vec![0x42]);
        let bytes = item.to_bytes();
        let parsed = CollectionItem::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.iv, MAX_IV);
    }

    #[test]
    fn test_reader_writer() {
        let item = CollectionItem::new(42, vec![0x11, 0x22, 0x33, 0x44, 0x55]);

        // Test container framing with reader/writer
        let mut buf = Vec::new();
        item.write_to(&mut buf).unwrap();

        let mut cursor = Cursor::new(&buf);
        let parsed = CollectionItem::read_from(&mut cursor, buf.len()).unwrap();
        assert_eq!(parsed, item);

        // Test NanoTDF payload framing with reader/writer
        let mut buf2 = Vec::new();
        item.write_nanotdf_payload_to(&mut buf2).unwrap();

        let mut cursor2 = Cursor::new(&buf2);
        let parsed2 = CollectionItem::read_nanotdf_payload_from(&mut cursor2).unwrap();
        assert_eq!(parsed2, item);
    }

    #[test]
    fn test_constants() {
        assert_eq!(MAX_IV, 0x00FF_FFFF);
        assert_eq!(RESERVED_POLICY_IV, 0);
        assert_eq!(DEFAULT_ROTATION_THRESHOLD, 0x0080_0000);
    }
}
