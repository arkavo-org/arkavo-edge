//! Resource Locator for NanoTDF
//!
//! The Resource Locator provides a compact way to reference external resources
//! like KAS endpoints and remote policies.

use crate::binary::{BinaryRead, BinaryWrite, read_u8, write_u8};
use std::io::{self, Read, Write};

/// Protocol type for resource location
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Protocol {
    /// HTTP protocol
    Http = 0x0,
    /// HTTPS protocol
    Https = 0x1,
    /// Shared Resource Directory (experimental)
    SharedResourceDirectory = 0xF,
}

impl Protocol {
    /// Parse protocol from 4-bit value
    pub fn from_bits(bits: u8) -> io::Result<Self> {
        match bits & 0x0F {
            0x0 => Ok(Protocol::Http),
            0x1 => Ok(Protocol::Https),
            0xF => Ok(Protocol::SharedResourceDirectory),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unknown protocol: 0x{:X}", bits),
            )),
        }
    }

    /// Convert to 4-bit value
    pub fn to_bits(self) -> u8 {
        self as u8
    }
}

/// Identifier type and length
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentifierType {
    /// No identifier
    None,
    /// 2-byte identifier
    TwoByte,
    /// 8-byte identifier
    EightByte,
    /// 32-byte identifier
    ThirtyTwoByte,
}

impl IdentifierType {
    /// Parse identifier type from 4-bit value
    pub fn from_bits(bits: u8) -> io::Result<Self> {
        match (bits >> 4) & 0x0F {
            0x0 => Ok(IdentifierType::None),
            0x1 => Ok(IdentifierType::TwoByte),
            0x2 => Ok(IdentifierType::EightByte),
            0x3 => Ok(IdentifierType::ThirtyTwoByte),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unknown identifier type: 0x{:X}", bits >> 4),
            )),
        }
    }

    /// Convert to 4-bit value (upper nibble)
    pub fn to_bits(self) -> u8 {
        let value = match self {
            IdentifierType::None => 0x0,
            IdentifierType::TwoByte => 0x1,
            IdentifierType::EightByte => 0x2,
            IdentifierType::ThirtyTwoByte => 0x3,
        };
        value << 4
    }

    /// Get the byte length of this identifier type
    pub fn byte_length(self) -> usize {
        match self {
            IdentifierType::None => 0,
            IdentifierType::TwoByte => 2,
            IdentifierType::EightByte => 8,
            IdentifierType::ThirtyTwoByte => 32,
        }
    }
}

/// Resource Locator - compact reference to external resources
///
/// Structure:
/// ```text
/// ┌────────────────┬──────────────┬────────────┬─────────────────────┐
/// │ Protocol (1B)  │ Body Len (1B)│ Body (var) │ Identifier (0-32B)  │
/// └────────────────┴──────────────┴────────────┴─────────────────────┘
/// ```
///
/// The Protocol byte contains:
/// - Bits 7-4: Identifier type
/// - Bits 3-0: Protocol enum
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLocator {
    /// Protocol type
    pub protocol: Protocol,
    /// Identifier type and optional value
    pub identifier: Option<Vec<u8>>,
    /// Resource body (e.g., "kas.example.com/kas" for HTTPS)
    pub body: Vec<u8>,
}

impl ResourceLocator {
    /// Create a new resource locator
    pub fn new(protocol: Protocol, body: impl Into<Vec<u8>>) -> Self {
        Self {
            protocol,
            identifier: None,
            body: body.into(),
        }
    }

    /// Set the identifier
    pub fn with_identifier(mut self, identifier: Vec<u8>) -> io::Result<Self> {
        // Validate identifier length
        match identifier.len() {
            0 => {
                self.identifier = None;
            }
            2 | 8 | 32 => {
                self.identifier = Some(identifier);
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Invalid identifier length: {}", identifier.len()),
                ));
            }
        }
        Ok(self)
    }

    /// Get the identifier type
    pub fn identifier_type(&self) -> IdentifierType {
        match &self.identifier {
            None => IdentifierType::None,
            Some(id) => match id.len() {
                2 => IdentifierType::TwoByte,
                8 => IdentifierType::EightByte,
                32 => IdentifierType::ThirtyTwoByte,
                _ => IdentifierType::None, // Should never happen due to validation
            },
        }
    }

    /// Create from URL string (convenience method)
    pub fn from_url(url: &str) -> io::Result<Self> {
        if let Some(rest) = url.strip_prefix("http://") {
            Ok(Self::new(Protocol::Http, rest.as_bytes()))
        } else if let Some(rest) = url.strip_prefix("https://") {
            Ok(Self::new(Protocol::Https, rest.as_bytes()))
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "URL must start with http:// or https://",
            ))
        }
    }

    /// Convert to URL string (convenience method)
    pub fn to_url(&self) -> io::Result<String> {
        let protocol_str = match self.protocol {
            Protocol::Http => "http://",
            Protocol::Https => "https://",
            Protocol::SharedResourceDirectory => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Shared Resource Directory cannot be converted to URL",
                ));
            }
        };

        let body_str = String::from_utf8(self.body.clone()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "Body contains invalid UTF-8")
        })?;

        Ok(format!("{}{}", protocol_str, body_str))
    }
}

impl BinaryRead for ResourceLocator {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<Self> {
        // Read protocol header (1 byte)
        let header = read_u8(reader)?;

        // Parse protocol and identifier type
        let protocol = Protocol::from_bits(header)?;
        let identifier_type = IdentifierType::from_bits(header)?;

        // Read body length (1 byte)
        let body_len = read_u8(reader)? as usize;

        // Read body
        let mut body = vec![0u8; body_len];
        reader.read_exact(&mut body)?;

        // Read identifier if present
        let identifier = if identifier_type.byte_length() > 0 {
            let mut id = vec![0u8; identifier_type.byte_length()];
            reader.read_exact(&mut id)?;
            Some(id)
        } else {
            None
        };

        Ok(Self {
            protocol,
            identifier,
            body,
        })
    }
}

impl BinaryWrite for ResourceLocator {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        // Validate body length
        if self.body.len() > 255 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Body too long: {} bytes (max 255)", self.body.len()),
            ));
        }

        // Write protocol header (protocol + identifier type)
        let header = self.identifier_type().to_bits() | self.protocol.to_bits();
        write_u8(writer, header)?;

        // Write body length
        write_u8(writer, self.body.len() as u8)?;

        // Write body
        writer.write_all(&self.body)?;

        // Write identifier if present
        if let Some(ref id) = self.identifier {
            writer.write_all(id)?;
        }

        Ok(())
    }

    fn serialized_size(&self) -> usize {
        1 + // Protocol header
        1 + // Body length
        self.body.len() +
        self.identifier.as_ref().map(|id| id.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_resource_locator_http() {
        let locator = ResourceLocator::from_url("http://kas.example.com").unwrap();
        assert_eq!(locator.protocol, Protocol::Http);
        assert_eq!(locator.body, b"kas.example.com");
        assert_eq!(locator.identifier, None);
    }

    #[test]
    fn test_resource_locator_https() {
        let locator = ResourceLocator::from_url("https://kas.example.com/kas").unwrap();
        assert_eq!(locator.protocol, Protocol::Https);
        assert_eq!(locator.body, b"kas.example.com/kas");
    }

    #[test]
    fn test_resource_locator_roundtrip() {
        let original = ResourceLocator::new(Protocol::Https, b"kas.virtru.com".to_vec());

        let mut buf = Vec::new();
        original.write_to(&mut buf).unwrap();

        let mut cursor = Cursor::new(buf);
        let decoded = ResourceLocator::read_from(&mut cursor).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_resource_locator_with_identifier() {
        let original = ResourceLocator::new(Protocol::Https, b"kas.example.com".to_vec())
            .with_identifier(vec![0x01, 0x02])
            .unwrap();

        assert_eq!(original.identifier_type(), IdentifierType::TwoByte);

        let mut buf = Vec::new();
        original.write_to(&mut buf).unwrap();

        let mut cursor = Cursor::new(buf);
        let decoded = ResourceLocator::read_from(&mut cursor).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_to_url() {
        let locator = ResourceLocator::new(Protocol::Https, b"kas.example.com/kas".to_vec());
        assert_eq!(locator.to_url().unwrap(), "https://kas.example.com/kas");
    }
}
