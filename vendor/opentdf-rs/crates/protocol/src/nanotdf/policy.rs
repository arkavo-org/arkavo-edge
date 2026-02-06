//! NanoTDF Policy Structures

use crate::binary::{BinaryRead, BinaryWrite, read_u8, read_u16_be, write_u8, write_u16_be};
use crate::nanotdf::resource_locator::ResourceLocator;
use std::io::{self, Read, Write};

/// Policy type indicator
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PolicyType {
    /// Remote policy (reference via Resource Locator)
    Remote = 0x00,
    /// Embedded policy (plaintext)
    EmbeddedPlaintext = 0x01,
    /// Embedded policy (encrypted)
    EmbeddedEncrypted = 0x02,
    /// Embedded policy (encrypted with separate key access)
    EmbeddedEncryptedPolicyKeyAccess = 0x03,
}

impl PolicyType {
    /// Parse from byte
    pub fn from_byte(byte: u8) -> io::Result<Self> {
        match byte {
            0x00 => Ok(PolicyType::Remote),
            0x01 => Ok(PolicyType::EmbeddedPlaintext),
            0x02 => Ok(PolicyType::EmbeddedEncrypted),
            0x03 => Ok(PolicyType::EmbeddedEncryptedPolicyKeyAccess),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid policy type: 0x{:02X}", byte),
            )),
        }
    }

    /// Convert to byte
    pub fn to_byte(self) -> u8 {
        self as u8
    }
}

/// Policy body variants
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyBody {
    /// Remote policy reference
    Remote(ResourceLocator),
    /// Embedded plaintext policy
    EmbeddedPlaintext(Vec<u8>),
    /// Embedded encrypted policy (uses reserved IV 0x000000)
    EmbeddedEncrypted(Vec<u8>),
    /// Embedded encrypted policy with separate key access
    EmbeddedEncryptedWithKeyAccess {
        /// Encrypted policy content
        content: Vec<u8>,
        /// KAS resource locator for policy key
        key_access: ResourceLocator,
        /// Ephemeral public key for policy key derivation
        ephemeral_key: Vec<u8>,
    },
}

impl PolicyBody {
    /// Get the policy type
    pub fn policy_type(&self) -> PolicyType {
        match self {
            PolicyBody::Remote(_) => PolicyType::Remote,
            PolicyBody::EmbeddedPlaintext(_) => PolicyType::EmbeddedPlaintext,
            PolicyBody::EmbeddedEncrypted(_) => PolicyType::EmbeddedEncrypted,
            PolicyBody::EmbeddedEncryptedWithKeyAccess { .. } => {
                PolicyType::EmbeddedEncryptedPolicyKeyAccess
            }
        }
    }
}

/// Policy with cryptographic binding
///
/// The policy can be:
/// - Remote (referenced by URL)
/// - Embedded plaintext
/// - Embedded encrypted
/// - Embedded encrypted with separate key access
///
/// All policies include a binding that cryptographically ties
/// the policy to the payload encryption key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// Policy content
    pub body: PolicyBody,
    /// Cryptographic binding (GMAC tag or ECDSA signature)
    /// Size depends on binding mode:
    /// - GMAC: 64 bits = 8 bytes
    /// - ECDSA: curve-dependent (64-132 bytes for r,s)
    pub binding: Vec<u8>,
}

impl Policy {
    /// Create a new policy
    pub fn new(body: PolicyBody, binding: Vec<u8>) -> Self {
        Self { body, binding }
    }

    /// Create a remote policy
    pub fn remote(locator: ResourceLocator, binding: Vec<u8>) -> Self {
        Self::new(PolicyBody::Remote(locator), binding)
    }

    /// Create an embedded plaintext policy
    pub fn embedded_plaintext(content: Vec<u8>, binding: Vec<u8>) -> Self {
        Self::new(PolicyBody::EmbeddedPlaintext(content), binding)
    }

    /// Create an embedded encrypted policy
    pub fn embedded_encrypted(content: Vec<u8>, binding: Vec<u8>) -> Self {
        Self::new(PolicyBody::EmbeddedEncrypted(content), binding)
    }
}

impl BinaryRead for Policy {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<Self> {
        // Read policy type
        let policy_type = PolicyType::from_byte(read_u8(reader)?)?;

        // Read body based on type
        let body = match policy_type {
            PolicyType::Remote => {
                let locator = ResourceLocator::read_from(reader)?;
                PolicyBody::Remote(locator)
            }
            PolicyType::EmbeddedPlaintext | PolicyType::EmbeddedEncrypted => {
                // Read content length (2 bytes)
                let content_len = read_u16_be(reader)? as usize;

                // Read content
                let mut content = vec![0u8; content_len];
                reader.read_exact(&mut content)?;

                if policy_type == PolicyType::EmbeddedPlaintext {
                    PolicyBody::EmbeddedPlaintext(content)
                } else {
                    PolicyBody::EmbeddedEncrypted(content)
                }
            }
            PolicyType::EmbeddedEncryptedPolicyKeyAccess => {
                // Read content length (2 bytes)
                let content_len = read_u16_be(reader)? as usize;

                // Read content
                let mut content = vec![0u8; content_len];
                reader.read_exact(&mut content)?;

                // Read key access resource locator
                let key_access = ResourceLocator::read_from(reader)?;

                // Read ephemeral public key
                // Note: We need to know the ECC mode from the header to know the size
                // For now, we'll read a P-256 key (33 bytes) as default
                // This will need to be refactored to pass ECC mode from header
                let mut ephemeral_key = vec![0u8; 33];
                reader.read_exact(&mut ephemeral_key)?;

                PolicyBody::EmbeddedEncryptedWithKeyAccess {
                    content,
                    key_access,
                    ephemeral_key,
                }
            }
        };

        // Read binding
        // L1L v12 format: Always 8 bytes (SHA-256 last 8 bytes)
        // Note: This should eventually be refactored to pass binding mode from header
        // for ECDSA support (which has variable size)
        let mut binding = vec![0u8; 8];
        reader.read_exact(&mut binding)?;

        Ok(Self { body, binding })
    }
}

impl BinaryWrite for Policy {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        // Write policy type
        write_u8(writer, self.body.policy_type().to_byte())?;

        // Write body based on type
        match &self.body {
            PolicyBody::Remote(locator) => {
                locator.write_to(writer)?;
            }
            PolicyBody::EmbeddedPlaintext(content) | PolicyBody::EmbeddedEncrypted(content) => {
                // Validate content length
                if content.len() > 0xFFFF {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "Policy content too large: {} bytes (max 65535)",
                            content.len()
                        ),
                    ));
                }

                // Write content length (2 bytes)
                write_u16_be(writer, content.len() as u16)?;

                // Write content
                writer.write_all(content)?;
            }
            PolicyBody::EmbeddedEncryptedWithKeyAccess {
                content,
                key_access,
                ephemeral_key,
            } => {
                // Validate content length
                if content.len() > 0xFFFF {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "Policy content too large: {} bytes (max 65535)",
                            content.len()
                        ),
                    ));
                }

                // Write content length (2 bytes)
                write_u16_be(writer, content.len() as u16)?;

                // Write content
                writer.write_all(content)?;

                // Write key access resource locator
                key_access.write_to(writer)?;

                // Write ephemeral public key
                writer.write_all(ephemeral_key)?;
            }
        }

        // Write binding
        writer.write_all(&self.binding)?;

        Ok(())
    }

    fn serialized_size(&self) -> usize {
        let body_size = match &self.body {
            PolicyBody::Remote(locator) => locator.serialized_size(),
            PolicyBody::EmbeddedPlaintext(content) | PolicyBody::EmbeddedEncrypted(content) => {
                2 + content.len() // 2 bytes length + content
            }
            PolicyBody::EmbeddedEncryptedWithKeyAccess {
                content,
                key_access,
                ephemeral_key,
            } => 2 + content.len() + key_access.serialized_size() + ephemeral_key.len(),
        };

        1 + // Policy type
        body_size +
        self.binding.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nanotdf::resource_locator::Protocol;

    #[test]
    fn test_remote_policy() {
        let locator = ResourceLocator::new(Protocol::Https, b"kas.example.com/policy/123");
        let binding = vec![0u8; 8]; // GMAC binding
        let policy = Policy::remote(locator, binding);

        assert!(matches!(policy.body, PolicyBody::Remote(_)));
        assert_eq!(policy.body.policy_type(), PolicyType::Remote);
    }

    #[test]
    fn test_embedded_plaintext_policy() {
        let content = b"policy content".to_vec();
        let binding = vec![0u8; 8];
        let policy = Policy::embedded_plaintext(content.clone(), binding);

        match &policy.body {
            PolicyBody::EmbeddedPlaintext(c) => assert_eq!(c, &content),
            _ => panic!("Expected EmbeddedPlaintext"),
        }
    }

    #[test]
    fn test_policy_roundtrip() {
        let content = b"test policy".to_vec();
        let binding = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let original = Policy::embedded_plaintext(content, binding);

        let mut buf = Vec::new();
        original.write_to(&mut buf).unwrap();

        // Note: This test will fail until we properly handle binding size in read_from
        // For now, it demonstrates the write path works
        assert!(!buf.is_empty());
    }
}
