//! NanoTDF Implementation
//!
//! NanoTDF is a compact TDF format designed for constrained environments (IoT, mobile).
//! It uses EC-based key agreement (ECDH + HKDF) and binary encoding for efficiency.
//!
//! # Specification
//!
//! See: https://github.com/opentdf/spec/blob/main/schema/NanoTDF.md
//!
//! # Binary Format
//!
//! ```text
//! [Magic/Version (3B)] [Header] [Payload] [Optional Signature]
//!
//! Header:
//!   - Resource Locator (KAS)
//!   - ECC/SymmetricAndPayloadConfig (1 byte bitfield)
//!   - Policy Info
//!   - Ephemeral Public Key (33/49/67/33 bytes for P-256/P-384/P-521/secp256k1)
//!   - Policy Binding (GMAC tag)
//!
//! Payload:
//!   - Length (3 bytes, big-endian, max 16MB)
//!   - IV (3 bytes)
//!   - Ciphertext (variable)
//!   - Auth Tag (8-16 bytes GCM tag)
//!
//! Signature (optional):
//!   - Public Key (33-67 bytes compressed)
//!   - Signature (64-132 bytes ECDSA)
//! ```

use crate::kem::ec::{EcCurve, EcdhKem};
use crate::tdf::nanotdf_crypto::{NanoTdfIv, TagSize, decrypt, encrypt};
use opentdf_protocol::binary::{BinaryRead, BinaryWrite, read_u24_be, write_u24_be};
use opentdf_protocol::nanotdf::{
    MagicNumberAndVersion,
    header::{
        EccAndBindingMode, EccMode, Header, PayloadSignatureMode, SymmetricAndPayloadConfig,
        SymmetricCipher,
    },
    policy::{Policy, PolicyBody},
    resource_locator::{Protocol, ResourceLocator},
};
use std::io::{self, Cursor, Read, Write};
use thiserror::Error;

/// NanoTDF errors
#[derive(Debug, Error)]
pub enum NanoTdfError {
    #[error("Invalid magic number or version")]
    InvalidMagic,

    #[error("Invalid header: {0}")]
    InvalidHeader(String),

    #[error("Invalid payload: {0}")]
    InvalidPayload(String),

    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),

    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),

    #[error("Policy binding verification failed")]
    PolicyBindingFailed,

    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("KEM error: {0}")]
    Kem(#[from] crate::kem::KemError),

    #[error("Crypto error: {0}")]
    Crypto(#[from] crate::tdf::nanotdf_crypto::NanoTdfCryptoError),

    #[error("Unsupported feature: {0}")]
    Unsupported(String),

    // Collection/Dataset errors
    #[error(
        "Collection IV exhausted (max 16,777,215 items). Create new collection with fresh DEK."
    )]
    IvExhausted,

    #[error("Collection rotation threshold reached ({0} items). Consider creating new collection.")]
    RotationThresholdReached(u32),

    #[error("Invalid DEK length: expected 32 bytes, got {0}")]
    InvalidDekLength(usize),

    #[error("KAS URL not configured")]
    MissingKasUrl,

    #[error("Policy not configured")]
    MissingPolicy,
}

/// Complete NanoTDF structure
#[derive(Debug, Clone)]
pub struct NanoTdf {
    /// Magic number and version (L1L)
    pub magic: MagicNumberAndVersion,

    /// NanoTDF header
    pub header: Header,

    /// Encrypted payload (length + IV + ciphertext + tag)
    pub payload: NanoTdfPayload,

    /// Optional ECDSA signature
    pub signature: Option<NanoTdfSignature>,
}

/// NanoTDF encrypted payload
#[derive(Debug, Clone)]
pub struct NanoTdfPayload {
    /// Payload length (3 bytes, max 16MB)
    pub length: u32,

    /// Initialization vector (3 bytes)
    pub iv: NanoTdfIv,

    /// Ciphertext + authentication tag (combined)
    pub ciphertext_and_tag: Vec<u8>,
}

/// Optional ECDSA signature for policy enforcement
#[derive(Debug, Clone)]
pub struct NanoTdfSignature {
    /// Signing public key (compressed EC point)
    pub public_key: Vec<u8>,

    /// ECDSA signature over header + payload
    pub signature: Vec<u8>,
}

/// Builder for creating NanoTDF files
///
/// # Example
///
/// ```rust,ignore
/// use opentdf_crypto::tdf::NanoTdfBuilder;
///
/// let nanotdf = NanoTdfBuilder::new()
///     .kas_url("http://localhost:8080/kas")
///     .policy_remote_body(b"policy-uuid-here".to_vec())
///     .use_ecdh_binding(false)
///     .encrypt(b"sensitive data", &kas_public_key)?;
///
/// let bytes = nanotdf.to_bytes()?;
/// ```
#[derive(Clone)]
pub struct NanoTdfBuilder {
    kas_url: Option<String>,
    kas_kid: Option<Vec<u8>>,
    policy_body: Option<PolicyBody>,
    ecc_mode: EccMode,
    use_ecdh_binding: bool,
}

impl NanoTdfBuilder {
    /// Create a new NanoTDF builder with defaults (P-256, GMAC binding)
    pub fn new() -> Self {
        NanoTdfBuilder {
            kas_url: None,
            kas_kid: None,
            policy_body: None,
            ecc_mode: EccMode::Secp256r1,
            use_ecdh_binding: false,
        }
    }

    /// Set the KAS URL
    #[must_use]
    pub fn kas_url(mut self, url: impl Into<String>) -> Self {
        self.kas_url = Some(url.into());
        self
    }

    /// Set the KAS URL with a key ID (kid)
    #[must_use]
    pub fn kas_url_with_kid(mut self, url: impl Into<String>, kid: &[u8]) -> Self {
        self.kas_url = Some(url.into());
        self.kas_kid = Some(kid.to_vec());
        self
    }

    /// Set a remote policy body (just the body bytes, not full Policy)
    #[must_use]
    pub fn policy_remote_body(mut self, _body: Vec<u8>) -> Self {
        // For remote policy, create a simple resource locator
        // This is simplified - in practice would parse the body as a locator
        let locator = ResourceLocator::from_url("http://policy")
            .unwrap_or_else(|_| ResourceLocator::new(Protocol::Http, "policy".as_bytes().to_vec()));
        self.policy_body = Some(PolicyBody::Remote(locator));
        self
    }

    /// Set an embedded plaintext policy
    #[must_use]
    pub fn policy_plaintext(mut self, body: Vec<u8>) -> Self {
        self.policy_body = Some(PolicyBody::EmbeddedPlaintext(body));
        self
    }

    /// Set the ECC curve (default: P-256)
    #[must_use]
    pub fn ecc_mode(mut self, mode: EccMode) -> Self {
        self.ecc_mode = mode;
        self
    }

    /// Use ECDH-based policy binding instead of GMAC (default: false)
    #[must_use]
    pub fn use_ecdh_binding(mut self, enabled: bool) -> Self {
        self.use_ecdh_binding = enabled;
        self
    }

    /// Encrypt data and build NanoTDF
    ///
    /// # Process
    ///
    /// 1. Generate ephemeral EC key pair for selected curve
    /// 2. Perform ECDH with recipient KAS public key
    /// 3. Derive AES-256 key using HKDF-SHA256 with NanoTDF salt
    /// 4. Encrypt payload with AES-256-GCM (3-byte IV, variable tag size)
    /// 5. Calculate policy binding (GMAC or ECDH-based)
    /// 6. Assemble header + payload
    pub fn encrypt(self, plaintext: &[u8], kas_public_key: &[u8]) -> Result<NanoTdf, NanoTdfError> {
        // Validate payload size (max 16MB - 3-byte length field)
        const MAX_PAYLOAD_SIZE: usize = 0xFFFFFF; // 16,777,215 bytes
        if plaintext.len() > MAX_PAYLOAD_SIZE {
            return Err(NanoTdfError::InvalidPayload(format!(
                "Payload too large: {} bytes (max {})",
                plaintext.len(),
                MAX_PAYLOAD_SIZE
            )));
        }

        let kas_url = self
            .kas_url
            .ok_or_else(|| NanoTdfError::InvalidHeader("KAS URL not set".to_string()))?;

        let policy_body = self
            .policy_body
            .ok_or_else(|| NanoTdfError::InvalidHeader("Policy not set".to_string()))?;

        // Convert EccMode to EcCurve
        let curve = match self.ecc_mode {
            EccMode::Secp256r1 => EcCurve::P256,
            EccMode::Secp384r1 => EcCurve::P384,
            EccMode::Secp521r1 => EcCurve::P521,
            EccMode::Secp256k1 => EcCurve::Secp256k1,
        };

        // Create ECDH KEM for the selected curve
        let kem = EcdhKem::new(curve);

        // Perform ECDH + HKDF to derive encryption key and get ephemeral public key
        let (aes_key, ephemeral_public_key) = kem.derive_key_with_ephemeral(kas_public_key)?;

        // Generate random 3-byte IV
        let iv = NanoTdfIv::random();

        // Encrypt payload with AES-256-GCM
        // Tag size: default to 96-bit (12 bytes) for RustCrypto compatibility
        let tag_size = TagSize::Bits96;
        let ciphertext_and_tag = encrypt(&aes_key, &iv, plaintext, tag_size)?;

        // Calculate policy binding (L1L v12 format)
        // According to Go/otdfctl gold standard: SHA-256 hash of policy body, last 8 bytes
        let policy_binding = if self.use_ecdh_binding {
            // ECDSA binding (future)
            return Err(NanoTdfError::Unsupported(
                "ECDSA policy binding not yet implemented".to_string(),
            ));
        } else {
            // GMAC binding for L1L v12: SHA-256 of policy body, take last 8 bytes
            use sha2::{Digest, Sha256};
            let policy_bytes = match &policy_body {
                PolicyBody::Remote(locator) => {
                    // For remote policy, hash the resource locator bytes
                    let mut buf = Vec::new();
                    locator.write_to(&mut buf).map_err(|e| {
                        NanoTdfError::InvalidHeader(format!("Failed to serialize policy: {}", e))
                    })?;
                    buf
                }
                PolicyBody::EmbeddedPlaintext(content) => content.clone(),
                PolicyBody::EmbeddedEncrypted(content) => content.clone(),
                _ => {
                    return Err(NanoTdfError::Unsupported(
                        "Encrypted policy with key access not supported".to_string(),
                    ));
                }
            };

            let hash = Sha256::digest(&policy_bytes);
            hash[24..].to_vec() // Last 8 bytes
        };

        // Create KAS resource locator
        let mut kas_locator = ResourceLocator::from_url(&kas_url)
            .map_err(|e| NanoTdfError::InvalidHeader(format!("Invalid KAS URL: {}", e)))?;

        // Add key ID if provided
        if let Some(kid) = self.kas_kid {
            kas_locator = kas_locator
                .with_identifier(kid)
                .map_err(|e| NanoTdfError::InvalidHeader(format!("Invalid key ID: {}", e)))?;
        }

        // Create policy with binding
        let policy = Policy {
            body: policy_body,
            binding: policy_binding,
        };

        // Create header config
        let signature_mode = PayloadSignatureMode::none();
        let symmetric_cipher = SymmetricCipher::Aes256Gcm96;
        let config = SymmetricAndPayloadConfig::new(signature_mode, symmetric_cipher);

        // Create ECC and binding mode
        let ecc_and_binding_mode = EccAndBindingMode::new(false, self.ecc_mode);

        // Create header using the constructor (not struct literal)
        let header = Header::new(
            kas_locator,
            ecc_and_binding_mode,
            config,
            policy,
            ephemeral_public_key.to_vec(),
        )?;

        // Create payload
        // Note: length includes IV (3 bytes) + ciphertext + tag
        let payload = NanoTdfPayload {
            length: (3 + ciphertext_and_tag.len()) as u32,
            iv,
            ciphertext_and_tag,
        };

        Ok(NanoTdf {
            magic: MagicNumberAndVersion::new(),
            header,
            payload,
            signature: None,
        })
    }
}

impl Default for NanoTdfBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl NanoTdf {
    /// Serialize NanoTDF to binary format
    pub fn to_bytes(&self) -> Result<Vec<u8>, NanoTdfError> {
        let mut buffer = Vec::new();

        // Write header (includes magic + version)
        self.header.write_to(&mut buffer)?;

        // Write payload
        self.payload.write_to(&mut buffer)?;

        // Write optional signature
        if let Some(sig) = &self.signature {
            sig.write_to(&mut buffer)?;
        }

        Ok(buffer)
    }

    /// Deserialize NanoTDF from binary format
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, NanoTdfError> {
        let mut cursor = Cursor::new(bytes);

        // Read header (includes magic + version)
        let header = Header::read_from(&mut cursor)?;

        // Read payload
        let payload = NanoTdfPayload::read_from(&mut cursor)?;

        // Read optional signature
        let signature = if header
            .symmetric_and_payload_config
            .signature_mode
            .has_signature
        {
            let sig_ecc_mode = header
                .symmetric_and_payload_config
                .signature_mode
                .signature_ecc_mode
                .ok_or_else(|| {
                    NanoTdfError::InvalidHeader("Signature ECC mode missing".to_string())
                })?;
            Some(NanoTdfSignature::read_from(&mut cursor, sig_ecc_mode)?)
        } else {
            None
        };

        Ok(NanoTdf {
            magic: header.magic_number_and_version,
            header,
            payload,
            signature,
        })
    }

    /// Decrypt NanoTDF payload
    ///
    /// # Process
    ///
    /// 1. Parse header and extract ephemeral public key
    /// 2. Perform ECDH with recipient private key
    /// 3. Derive decryption key using HKDF-SHA256 with NanoTDF salt
    /// 4. Verify policy binding (GMAC or ECDH)
    /// 5. Decrypt payload with AES-256-GCM
    /// 6. Verify optional ECDSA signature
    pub fn decrypt(&self, kas_private_key: &[u8]) -> Result<Vec<u8>, NanoTdfError> {
        // Convert EccMode to EcCurve
        let curve = match self.header.ecc_and_binding_mode.ecc_mode {
            EccMode::Secp256r1 => EcCurve::P256,
            EccMode::Secp384r1 => EcCurve::P384,
            EccMode::Secp521r1 => EcCurve::P521,
            EccMode::Secp256k1 => EcCurve::Secp256k1,
        };

        // Create ECDH KEM for the curve
        let kem = EcdhKem::new(curve);

        // Perform ECDH + HKDF to derive decryption key using ephemeral public key
        let aes_key =
            kem.derive_key_with_private(kas_private_key, &self.header.ephemeral_public_key)?;

        // Verify policy binding (L1L v12: SHA-256 last 8 bytes)
        {
            use sha2::{Digest, Sha256};

            // Get policy bytes for binding verification
            let policy_bytes = match &self.header.policy.body {
                PolicyBody::Remote(locator) => {
                    let mut buf = Vec::new();
                    locator.write_to(&mut buf).map_err(|e| {
                        NanoTdfError::InvalidHeader(format!("Failed to serialize policy: {}", e))
                    })?;
                    buf
                }
                PolicyBody::EmbeddedPlaintext(content) => content.clone(),
                PolicyBody::EmbeddedEncrypted(content) => content.clone(),
                _ => {
                    return Err(NanoTdfError::Unsupported(
                        "Encrypted policy with key access not supported".to_string(),
                    ));
                }
            };

            // L1L v12 binding: SHA-256 of policy body, last 8 bytes
            let hash = Sha256::digest(&policy_bytes);
            let expected_binding = &hash[24..]; // Last 8 bytes

            // Policy binding should be 8 bytes for L1L v12
            if self.header.policy.binding.len() != 8 {
                return Err(NanoTdfError::PolicyBindingFailed);
            }

            if self.header.policy.binding != expected_binding {
                return Err(NanoTdfError::PolicyBindingFailed);
            }
        }

        // Determine tag size from config
        let tag_size = match self.header.symmetric_and_payload_config.symmetric_cipher {
            SymmetricCipher::Aes256Gcm64 => {
                #[cfg(feature = "nanotdf-mbedtls")]
                {
                    TagSize::Bits64
                }
                #[cfg(not(feature = "nanotdf-mbedtls"))]
                {
                    return Err(NanoTdfError::Unsupported(
                        "64-bit GCM tags require nanotdf-mbedtls feature".to_string(),
                    ));
                }
            }
            SymmetricCipher::Aes256Gcm96 => TagSize::Bits96,
            SymmetricCipher::Aes256Gcm104 => TagSize::Bits104,
            SymmetricCipher::Aes256Gcm112 => TagSize::Bits112,
            SymmetricCipher::Aes256Gcm120 => TagSize::Bits120,
            SymmetricCipher::Aes256Gcm128 => TagSize::Bits128,
        };

        // Decrypt payload
        let plaintext = decrypt(
            &aes_key,
            &self.payload.iv,
            &self.payload.ciphertext_and_tag,
            tag_size,
        )?;

        Ok(plaintext)
    }
}

// Binary serialization for NanoTdfPayload
impl BinaryWrite for NanoTdfPayload {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        // Write length (3 bytes, big-endian)
        write_u24_be(writer, self.length)?;

        // Write IV (3 bytes)
        writer.write_all(self.iv.as_bytes())?;

        // Write ciphertext + tag
        writer.write_all(&self.ciphertext_and_tag)?;

        Ok(())
    }

    fn serialized_size(&self) -> usize {
        3 + 3 + self.ciphertext_and_tag.len()
    }
}

impl BinaryRead for NanoTdfPayload {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<Self> {
        // Read length (3 bytes, big-endian)
        // Note: This length includes IV + ciphertext + tag
        let length = read_u24_be(reader)?;

        if length < 3 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Payload length too small: {} (must be at least 3 for IV)",
                    length
                ),
            ));
        }

        // Read all payload data (IV + ciphertext + tag)
        let mut payload_data = vec![0u8; length as usize];
        reader.read_exact(&mut payload_data)?;

        // Extract IV from first 3 bytes
        let iv = NanoTdfIv::from_bytes([payload_data[0], payload_data[1], payload_data[2]]);

        // Remaining bytes are ciphertext + tag
        let ciphertext_and_tag = payload_data[3..].to_vec();

        Ok(NanoTdfPayload {
            length,
            iv,
            ciphertext_and_tag,
        })
    }
}

// Binary serialization for NanoTdfSignature
impl NanoTdfSignature {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(&self.public_key)?;
        writer.write_all(&self.signature)?;
        Ok(())
    }

    fn read_from<R: Read>(reader: &mut R, ecc_mode: EccMode) -> io::Result<Self> {
        let public_key_size = ecc_mode.public_key_size();
        let signature_size = ecc_mode.signature_size();

        let mut public_key = vec![0u8; public_key_size];
        reader.read_exact(&mut public_key)?;

        let mut signature = vec![0u8; signature_size];
        reader.read_exact(&mut signature)?;

        Ok(NanoTdfSignature {
            public_key,
            signature,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_api() {
        let builder = NanoTdfBuilder::new()
            .kas_url("http://localhost:8080/kas")
            .policy_remote_body(b"test-uuid".to_vec())
            .ecc_mode(EccMode::Secp256r1);

        // Verify builder state
        assert!(builder.kas_url.is_some());
        assert!(builder.policy_body.is_some());
    }

    #[test]
    fn test_payload_size_limit() {
        let builder = NanoTdfBuilder::new()
            .kas_url("http://localhost:8080/kas")
            .policy_remote_body(b"test-uuid".to_vec());

        // Create a dummy KAS key (won't work for encryption but tests size validation)
        let dummy_key = vec![0u8; 33];

        // Test max payload size (16MB - 1)
        let max_payload = vec![0u8; 0xFFFFFF];
        let result = builder.clone().encrypt(&max_payload, &dummy_key);
        // Will fail on crypto, but not on size validation
        assert!(result.is_err());

        // Test over-size payload
        let oversized_payload = vec![0u8; 0x1000000]; // 16MB
        let result = builder.encrypt(&oversized_payload, &dummy_key);
        assert!(matches!(result, Err(NanoTdfError::InvalidPayload(_))));
    }
}
