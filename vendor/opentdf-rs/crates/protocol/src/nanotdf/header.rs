//! NanoTDF Header Structures
//!
//! The header contains all metadata needed to decrypt the NanoTDF payload.

use crate::binary::{BinaryRead, BinaryWrite, read_u8, write_u8};
use crate::nanotdf::{MAGIC_NUMBER_AND_VERSION, Policy, resource_locator::ResourceLocator};
use std::io::{self, Read, Write};

/// Magic number and version (3 bytes)
/// "L1L" in base64 = 0x4C314C
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MagicNumberAndVersion {
    pub bytes: [u8; 3],
}

impl Default for MagicNumberAndVersion {
    fn default() -> Self {
        Self {
            bytes: MAGIC_NUMBER_AND_VERSION,
        }
    }
}

impl MagicNumberAndVersion {
    /// Create a new magic number and version
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate that this matches the expected magic number and version
    pub fn validate(&self) -> io::Result<()> {
        if self.bytes != MAGIC_NUMBER_AND_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Invalid magic number and version: {:02X}{:02X}{:02X} (expected 4C314C)",
                    self.bytes[0], self.bytes[1], self.bytes[2]
                ),
            ));
        }
        Ok(())
    }
}

impl BinaryRead for MagicNumberAndVersion {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<Self> {
        let mut bytes = [0u8; 3];
        reader.read_exact(&mut bytes)?;
        let magic = Self { bytes };
        magic.validate()?;
        Ok(magic)
    }
}

impl BinaryWrite for MagicNumberAndVersion {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(&self.bytes)
    }

    fn serialized_size(&self) -> usize {
        3
    }
}

/// Elliptic Curve parameters for key agreement and signatures
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EccMode {
    /// secp256r1 (P-256) - NIST curve
    Secp256r1 = 0x00,
    /// secp384r1 (P-384) - NIST curve
    Secp384r1 = 0x01,
    /// secp521r1 (P-521) - NIST curve
    Secp521r1 = 0x02,
    /// secp256k1 - Bitcoin curve
    Secp256k1 = 0x03,
}

impl EccMode {
    /// Parse from 3-bit value
    pub fn from_bits(bits: u8) -> io::Result<Self> {
        match bits & 0x07 {
            0x00 => Ok(EccMode::Secp256r1),
            0x01 => Ok(EccMode::Secp384r1),
            0x02 => Ok(EccMode::Secp521r1),
            0x03 => Ok(EccMode::Secp256k1),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid ECC mode: {}", bits),
            )),
        }
    }

    /// Convert to 3-bit value
    pub fn to_bits(self) -> u8 {
        self as u8
    }

    /// Get the size of compressed public keys for this curve
    pub fn public_key_size(self) -> usize {
        match self {
            EccMode::Secp256r1 => 33, // 1 byte prefix + 32 bytes
            EccMode::Secp384r1 => 49, // 1 byte prefix + 48 bytes
            EccMode::Secp521r1 => 67, // 1 byte prefix + 66 bytes
            EccMode::Secp256k1 => 33, // 1 byte prefix + 32 bytes
        }
    }

    /// Get the size of ECDSA signatures for this curve
    pub fn signature_size(self) -> usize {
        match self {
            EccMode::Secp256r1 => 64,  // r (32 bytes) + s (32 bytes)
            EccMode::Secp384r1 => 96,  // r (48 bytes) + s (48 bytes)
            EccMode::Secp521r1 => 132, // r (66 bytes) + s (66 bytes)
            EccMode::Secp256k1 => 64,  // r (32 bytes) + s (32 bytes)
        }
    }
}

/// ECC and Binding Mode (1 byte bitfield)
///
/// ```text
/// ┌─────────────┬──────────┬──────────────────┐
/// │USE_ECDSA(1b)│UNUSED(4b)│ECC Params Enum(3b)│
/// └─────────────┴──────────┴──────────────────┘
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EccAndBindingMode {
    /// Use ECDSA for policy binding (true) or GMAC (false)
    pub use_ecdsa_binding: bool,
    /// ECC parameters to use for ephemeral key
    pub ecc_mode: EccMode,
}

impl EccAndBindingMode {
    /// Create a new ECC and binding mode
    pub fn new(use_ecdsa_binding: bool, ecc_mode: EccMode) -> Self {
        Self {
            use_ecdsa_binding,
            ecc_mode,
        }
    }

    /// Parse from byte
    pub fn from_byte(byte: u8) -> io::Result<Self> {
        let use_ecdsa_binding = (byte & 0x80) != 0;
        let ecc_mode = EccMode::from_bits(byte & 0x07)?;
        Ok(Self {
            use_ecdsa_binding,
            ecc_mode,
        })
    }

    /// Convert to byte
    pub fn to_byte(self) -> u8 {
        let ecdsa_bit = if self.use_ecdsa_binding { 0x80 } else { 0x00 };
        ecdsa_bit | self.ecc_mode.to_bits()
    }
}

impl BinaryRead for EccAndBindingMode {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<Self> {
        let byte = read_u8(reader)?;
        Self::from_byte(byte)
    }
}

impl BinaryWrite for EccAndBindingMode {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_u8(writer, self.to_byte())
    }

    fn serialized_size(&self) -> usize {
        1
    }
}

/// Symmetric cipher for payload encryption
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SymmetricCipher {
    /// AES-256-GCM with 64-bit authentication tag
    Aes256Gcm64 = 0x00,
    /// AES-256-GCM with 96-bit authentication tag
    Aes256Gcm96 = 0x01,
    /// AES-256-GCM with 104-bit authentication tag
    Aes256Gcm104 = 0x02,
    /// AES-256-GCM with 112-bit authentication tag
    Aes256Gcm112 = 0x03,
    /// AES-256-GCM with 120-bit authentication tag
    Aes256Gcm120 = 0x04,
    /// AES-256-GCM with 128-bit authentication tag
    Aes256Gcm128 = 0x05,
}

impl SymmetricCipher {
    /// Parse from 4-bit value
    pub fn from_bits(bits: u8) -> io::Result<Self> {
        match bits & 0x0F {
            0x00 => Ok(SymmetricCipher::Aes256Gcm64),
            0x01 => Ok(SymmetricCipher::Aes256Gcm96),
            0x02 => Ok(SymmetricCipher::Aes256Gcm104),
            0x03 => Ok(SymmetricCipher::Aes256Gcm112),
            0x04 => Ok(SymmetricCipher::Aes256Gcm120),
            0x05 => Ok(SymmetricCipher::Aes256Gcm128),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid symmetric cipher: {}", bits),
            )),
        }
    }

    /// Convert to 4-bit value
    pub fn to_bits(self) -> u8 {
        self as u8
    }

    /// Get the size of the authentication tag in bytes
    pub fn tag_size(self) -> usize {
        match self {
            SymmetricCipher::Aes256Gcm64 => 8,
            SymmetricCipher::Aes256Gcm96 => 12,
            SymmetricCipher::Aes256Gcm104 => 13,
            SymmetricCipher::Aes256Gcm112 => 14,
            SymmetricCipher::Aes256Gcm120 => 15,
            SymmetricCipher::Aes256Gcm128 => 16,
        }
    }
}

/// Payload signature mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PayloadSignatureMode {
    /// Whether a signature is present
    pub has_signature: bool,
    /// ECC mode for signature (if present)
    pub signature_ecc_mode: Option<EccMode>,
}

impl PayloadSignatureMode {
    /// Create mode without signature
    pub fn none() -> Self {
        Self {
            has_signature: false,
            signature_ecc_mode: None,
        }
    }

    /// Create mode with signature
    pub fn with_signature(ecc_mode: EccMode) -> Self {
        Self {
            has_signature: true,
            signature_ecc_mode: Some(ecc_mode),
        }
    }

    /// Get the signature size if present
    pub fn signature_size(&self) -> usize {
        if let Some(ecc_mode) = self.signature_ecc_mode {
            ecc_mode.public_key_size() + ecc_mode.signature_size()
        } else {
            0
        }
    }
}

/// Symmetric and Payload Config (1 byte bitfield)
///
/// ```text
/// ┌──────────────┬────────────────┬─────────────────────┐
/// │HAS_SIG(1b)   │Sig ECC Mode(3b)│Symmetric Cipher(4b) │
/// └──────────────┴────────────────┴─────────────────────┘
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymmetricAndPayloadConfig {
    pub signature_mode: PayloadSignatureMode,
    pub symmetric_cipher: SymmetricCipher,
}

impl SymmetricAndPayloadConfig {
    /// Create a new config
    pub fn new(signature_mode: PayloadSignatureMode, symmetric_cipher: SymmetricCipher) -> Self {
        Self {
            signature_mode,
            symmetric_cipher,
        }
    }

    /// Parse from byte
    pub fn from_byte(byte: u8) -> io::Result<Self> {
        let has_signature = (byte & 0x80) != 0;
        let signature_ecc_mode = if has_signature {
            Some(EccMode::from_bits((byte >> 4) & 0x07)?)
        } else {
            None
        };
        let symmetric_cipher = SymmetricCipher::from_bits(byte & 0x0F)?;

        Ok(Self {
            signature_mode: PayloadSignatureMode {
                has_signature,
                signature_ecc_mode,
            },
            symmetric_cipher,
        })
    }

    /// Convert to byte
    pub fn to_byte(self) -> u8 {
        let sig_bit = if self.signature_mode.has_signature {
            0x80
        } else {
            0x00
        };
        let sig_ecc = if let Some(ecc_mode) = self.signature_mode.signature_ecc_mode {
            ecc_mode.to_bits() << 4
        } else {
            0x00
        };
        sig_bit | sig_ecc | self.symmetric_cipher.to_bits()
    }
}

impl BinaryRead for SymmetricAndPayloadConfig {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<Self> {
        let byte = read_u8(reader)?;
        Self::from_byte(byte)
    }
}

impl BinaryWrite for SymmetricAndPayloadConfig {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_u8(writer, self.to_byte())
    }

    fn serialized_size(&self) -> usize {
        1
    }
}

/// NanoTDF Header
///
/// Contains all metadata needed to decrypt the payload:
/// - Magic number and version
/// - KAS location
/// - Encryption parameters
/// - Policy
/// - Ephemeral public key
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub magic_number_and_version: MagicNumberAndVersion,
    pub kas: ResourceLocator,
    pub ecc_and_binding_mode: EccAndBindingMode,
    pub symmetric_and_payload_config: SymmetricAndPayloadConfig,
    pub policy: Policy,
    pub ephemeral_public_key: Vec<u8>,
}

impl Header {
    /// Create a new header
    pub fn new(
        kas: ResourceLocator,
        ecc_and_binding_mode: EccAndBindingMode,
        symmetric_and_payload_config: SymmetricAndPayloadConfig,
        policy: Policy,
        ephemeral_public_key: Vec<u8>,
    ) -> io::Result<Self> {
        // Validate ephemeral public key size
        let expected_size = ecc_and_binding_mode.ecc_mode.public_key_size();
        if ephemeral_public_key.len() != expected_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Invalid ephemeral public key size: {} (expected {})",
                    ephemeral_public_key.len(),
                    expected_size
                ),
            ));
        }

        Ok(Self {
            magic_number_and_version: MagicNumberAndVersion::new(),
            kas,
            ecc_and_binding_mode,
            symmetric_and_payload_config,
            policy,
            ephemeral_public_key,
        })
    }
}

impl BinaryRead for Header {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<Self> {
        let magic_number_and_version = MagicNumberAndVersion::read_from(reader)?;
        let kas = ResourceLocator::read_from(reader)?;
        let ecc_and_binding_mode = EccAndBindingMode::read_from(reader)?;
        let symmetric_and_payload_config = SymmetricAndPayloadConfig::read_from(reader)?;
        let policy = Policy::read_from(reader)?;

        // Read ephemeral public key (size depends on ECC mode)
        let key_size = ecc_and_binding_mode.ecc_mode.public_key_size();
        let mut ephemeral_public_key = vec![0u8; key_size];
        reader.read_exact(&mut ephemeral_public_key)?;

        Ok(Self {
            magic_number_and_version,
            kas,
            ecc_and_binding_mode,
            symmetric_and_payload_config,
            policy,
            ephemeral_public_key,
        })
    }
}

impl BinaryWrite for Header {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.magic_number_and_version.write_to(writer)?;
        self.kas.write_to(writer)?;
        self.ecc_and_binding_mode.write_to(writer)?;
        self.symmetric_and_payload_config.write_to(writer)?;
        self.policy.write_to(writer)?;
        writer.write_all(&self.ephemeral_public_key)?;
        Ok(())
    }

    fn serialized_size(&self) -> usize {
        self.magic_number_and_version.serialized_size()
            + self.kas.serialized_size()
            + self.ecc_and_binding_mode.serialized_size()
            + self.symmetric_and_payload_config.serialized_size()
            + self.policy.serialized_size()
            + self.ephemeral_public_key.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_magic_number() {
        assert_eq!(MAGIC_NUMBER_AND_VERSION, [0x4C, 0x31, 0x4C]);
        let magic = MagicNumberAndVersion::new();
        assert_eq!(magic.bytes, [0x4C, 0x31, 0x4C]);
        assert!(magic.validate().is_ok());
    }

    #[test]
    fn test_ecc_mode_sizes() {
        assert_eq!(EccMode::Secp256r1.public_key_size(), 33);
        assert_eq!(EccMode::Secp384r1.public_key_size(), 49);
        assert_eq!(EccMode::Secp521r1.public_key_size(), 67);
        assert_eq!(EccMode::Secp256k1.public_key_size(), 33);

        assert_eq!(EccMode::Secp256r1.signature_size(), 64);
        assert_eq!(EccMode::Secp384r1.signature_size(), 96);
        assert_eq!(EccMode::Secp521r1.signature_size(), 132);
    }

    #[test]
    fn test_ecc_and_binding_mode() {
        let mode = EccAndBindingMode::new(true, EccMode::Secp256r1);
        let byte = mode.to_byte();
        assert_eq!(byte, 0x80); // 1000 0000

        let parsed = EccAndBindingMode::from_byte(byte).unwrap();
        assert_eq!(parsed, mode);
    }

    #[test]
    fn test_symmetric_cipher_tag_sizes() {
        assert_eq!(SymmetricCipher::Aes256Gcm64.tag_size(), 8);
        assert_eq!(SymmetricCipher::Aes256Gcm96.tag_size(), 12);
        assert_eq!(SymmetricCipher::Aes256Gcm128.tag_size(), 16);
    }

    #[test]
    fn test_symmetric_and_payload_config() {
        let config = SymmetricAndPayloadConfig::new(
            PayloadSignatureMode::with_signature(EccMode::Secp256r1),
            SymmetricCipher::Aes256Gcm128,
        );
        let byte = config.to_byte();
        // Has signature (1) + ECC mode 0 (000) + Cipher 5 (0101)
        // 1000 0101 = 0x85
        assert_eq!(byte, 0x85);

        let parsed = SymmetricAndPayloadConfig::from_byte(byte).unwrap();
        assert_eq!(parsed.signature_mode.has_signature, true);
        assert_eq!(
            parsed.signature_mode.signature_ecc_mode,
            Some(EccMode::Secp256r1)
        );
        assert_eq!(parsed.symmetric_cipher, SymmetricCipher::Aes256Gcm128);
    }
}
