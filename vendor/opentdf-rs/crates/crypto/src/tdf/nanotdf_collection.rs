//! NanoTDF Collection/Dataset Implementation
//!
//! This module implements NanoTDF Collection support, enabling multiple encrypted
//! payloads to share a single NanoTDF header (manifest) and DEK.
//!
//! # Overview
//!
//! A NanoTDF Collection is a sequence of NanoTDF-protected payloads that share:
//! - The same NanoTDF header (KAS URL, ECC parameters, policy, etc.)
//! - A single DEK derived once from ECDH + HKDF
//!
//! Each Collection item differs only by:
//! - A unique IV (counter-based, encoded as 3 bytes)
//! - Its ciphertext + MAC
//!
//! # Use Cases
//!
//! - **NTDF-RTMP**: Live streaming with end-to-end encryption
//! - **IoT Data Streams**: Multiple sensor readings under one policy
//! - **Batch Processing**: Encrypt multiple payloads efficiently
//!
//! # Example
//!
//! ```rust,ignore
//! use opentdf_crypto::tdf::{NanoTdfCollectionBuilder, NanoTdfCollectionDecryptor};
//!
//! // Create collection (ECDH + HKDF done once)
//! let collection = NanoTdfCollectionBuilder::new()
//!     .kas_url("https://kas.example.com/kas")
//!     .policy_plaintext(policy.as_bytes().to_vec())
//!     .build(&kas_public_key)?;
//!
//! // Send manifest once
//! let header_bytes = collection.to_header_bytes()?;
//! send_manifest(&header_bytes);
//!
//! // Encrypt items (thread-safe via atomic IV counter)
//! for frame in video_frames {
//!     let item = collection.encrypt_item(&frame)?;
//!     send_frame(&item.to_bytes());
//! }
//! ```

use crate::kem::ec::{EcCurve, EcdhKem};
use crate::tdf::nanotdf::NanoTdfError;
use crate::tdf::nanotdf_crypto::{NanoTdfIv, TagSize, decrypt, encrypt};
use crate::types::AesKey;
use opentdf_protocol::binary::BinaryWrite;
use opentdf_protocol::nanotdf::{
    collection::{CollectionItem, DEFAULT_ROTATION_THRESHOLD, MAX_IV},
    header::{
        EccAndBindingMode, EccMode, Header, PayloadSignatureMode, SymmetricAndPayloadConfig,
        SymmetricCipher,
    },
    policy::{Policy, PolicyBody},
    resource_locator::ResourceLocator,
};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::sync::atomic::{AtomicU32, Ordering};

/// NanoTDF Collection for encrypting multiple payloads with a shared DEK
///
/// The collection uses an atomic counter for IV allocation, making it safe
/// to call `encrypt_item()` concurrently from multiple threads.
///
/// # Thread Safety
///
/// The IV counter uses atomic compare-and-swap (CAS) operations to ensure
/// unique IV allocation even under concurrent access. The DEK and header
/// are immutable after construction.
pub struct NanoTdfCollection {
    /// The NanoTDF header (contains KAS, policy, ephemeral public key)
    header: Header,

    /// Derived AES-256 encryption key (from ECDH + HKDF)
    dek: AesKey,

    /// Atomic IV counter: starts at 1 (0 is reserved for encrypted policy)
    /// Uses CAS to ensure unique, monotonically increasing IVs
    iv_counter: AtomicU32,

    /// Configurable rotation threshold for early warning
    rotation_threshold: u32,

    /// Tag size derived from header's symmetric cipher config
    tag_size: TagSize,
}

impl NanoTdfCollection {
    /// Get the current IV counter value
    pub fn current_iv(&self) -> u32 {
        self.iv_counter.load(Ordering::Relaxed)
    }

    /// Get remaining capacity before IV exhaustion
    pub fn remaining_capacity(&self) -> u32 {
        let current = self.current_iv();
        if current > MAX_IV {
            0
        } else {
            MAX_IV - current + 1 // +1 because current hasn't been used yet
        }
    }

    /// Check if rotation threshold has been reached
    ///
    /// This provides an early warning signal before IV exhaustion.
    pub fn rotation_threshold_reached(&self) -> bool {
        self.current_iv() >= self.rotation_threshold
    }

    /// Check if IV space is exhausted
    ///
    /// When true, no more items can be encrypted. Create a new collection.
    pub fn is_exhausted(&self) -> bool {
        self.current_iv() > MAX_IV
    }

    /// Get reference to the header
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Serialize the header for transmission
    ///
    /// This is the "manifest" that receivers need to decrypt collection items.
    /// Send this once at the start of a stream.
    pub fn to_header_bytes(&self) -> Result<Vec<u8>, NanoTdfError> {
        let mut buf = Vec::with_capacity(self.header.serialized_size());
        self.header.write_to(&mut buf)?;
        Ok(buf)
    }

    /// Encrypt a single item with the next available IV
    ///
    /// This method is thread-safe. It atomically allocates a unique IV
    /// and encrypts the plaintext.
    ///
    /// # Errors
    ///
    /// Returns `NanoTdfError::IvExhausted` if the IV counter has reached MAX_IV.
    pub fn encrypt_item(&self, plaintext: &[u8]) -> Result<CollectionItem, NanoTdfError> {
        // Atomically allocate next IV using CAS
        let iv = self.allocate_iv()?;

        // Create NanoTdfIv from counter
        let nano_iv = NanoTdfIv::from_bytes([
            ((iv >> 16) & 0xFF) as u8,
            ((iv >> 8) & 0xFF) as u8,
            (iv & 0xFF) as u8,
        ]);

        // Encrypt payload with AES-256-GCM
        let ciphertext_and_tag = encrypt(&self.dek, &nano_iv, plaintext, self.tag_size)?;

        Ok(CollectionItem::new(iv, ciphertext_and_tag))
    }

    /// Decrypt a collection item using the cached DEK
    ///
    /// This is primarily for KAS-side or testing scenarios where you have
    /// the collection that was used for encryption.
    pub fn decrypt_item(&self, item: &CollectionItem) -> Result<Vec<u8>, NanoTdfError> {
        // Create NanoTdfIv from item's IV counter
        let nano_iv = NanoTdfIv::from_bytes([
            ((item.iv >> 16) & 0xFF) as u8,
            ((item.iv >> 8) & 0xFF) as u8,
            (item.iv & 0xFF) as u8,
        ]);

        // Decrypt payload
        let plaintext = decrypt(&self.dek, &nano_iv, &item.ciphertext_and_tag, self.tag_size)?;

        Ok(plaintext)
    }

    /// Atomically allocate the next IV using compare-and-swap
    ///
    /// This ensures unique IV allocation even under concurrent access.
    fn allocate_iv(&self) -> Result<u32, NanoTdfError> {
        loop {
            let current = self.iv_counter.load(Ordering::Relaxed);

            // Check if exhausted
            if current > MAX_IV {
                return Err(NanoTdfError::IvExhausted);
            }

            let next = current + 1;

            // Attempt CAS - if another thread beat us, retry
            match self.iv_counter.compare_exchange(
                current,
                next,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Ok(current),
                Err(_) => continue, // Another thread incremented, retry
            }
        }
    }

    /// Get the tag size for this collection
    pub fn tag_size(&self) -> TagSize {
        self.tag_size
    }

    /// Get the authentication tag size in bytes
    pub fn tag_size_bytes(&self) -> usize {
        self.tag_size.bytes()
    }
}

impl std::fmt::Debug for NanoTdfCollection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NanoTdfCollection")
            .field("current_iv", &self.current_iv())
            .field("remaining_capacity", &self.remaining_capacity())
            .field("rotation_threshold", &self.rotation_threshold)
            .field("tag_size", &self.tag_size)
            .field("is_exhausted", &self.is_exhausted())
            .finish_non_exhaustive()
    }
}

/// Builder for creating NanoTDF collections
///
/// Follows the same pattern as `NanoTdfBuilder` for consistency.
#[derive(Clone)]
pub struct NanoTdfCollectionBuilder {
    kas_url: Option<String>,
    kas_kid: Option<Vec<u8>>,
    policy_body: Option<PolicyBody>,
    ecc_mode: EccMode,
    use_ecdh_binding: bool,
    rotation_threshold: Option<u32>,
    symmetric_cipher: SymmetricCipher,
}

impl NanoTdfCollectionBuilder {
    /// Create a new collection builder with defaults (P-256, GMAC binding, 96-bit tag)
    pub fn new() -> Self {
        Self {
            kas_url: None,
            kas_kid: None,
            policy_body: None,
            ecc_mode: EccMode::Secp256r1,
            use_ecdh_binding: false,
            rotation_threshold: None,
            symmetric_cipher: SymmetricCipher::Aes256Gcm96,
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

    /// Set a remote policy reference
    ///
    /// The `url` parameter specifies the policy resource locator URL
    /// (e.g., `"https://policy.example.com/policies/abc123"`).
    #[must_use]
    pub fn policy_remote(mut self, url: &str) -> Self {
        let locator = ResourceLocator::from_url(url).unwrap_or_else(|_| {
            ResourceLocator::new(
                opentdf_protocol::nanotdf::Protocol::Https,
                url.as_bytes().to_vec(),
            )
        });
        self.policy_body = Some(PolicyBody::Remote(locator));
        self
    }

    /// Set an embedded plaintext policy
    #[must_use]
    pub fn policy_plaintext(mut self, body: Vec<u8>) -> Self {
        self.policy_body = Some(PolicyBody::EmbeddedPlaintext(body));
        self
    }

    /// Set an embedded encrypted policy
    ///
    /// Note: The policy is encrypted using IV 0x000000 (reserved for policy).
    #[must_use]
    pub fn policy_encrypted(mut self, body: Vec<u8>) -> Self {
        self.policy_body = Some(PolicyBody::EmbeddedEncrypted(body));
        self
    }

    /// Set an embedded encrypted policy with separate key access
    #[must_use]
    pub fn policy_encrypted_with_key_access(
        mut self,
        content: Vec<u8>,
        key_access: ResourceLocator,
        ephemeral_key: Vec<u8>,
    ) -> Self {
        self.policy_body = Some(PolicyBody::EmbeddedEncryptedWithKeyAccess {
            content,
            key_access,
            ephemeral_key,
        });
        self
    }

    /// Set the ECC curve (default: P-256)
    #[must_use]
    pub fn ecc_mode(mut self, mode: EccMode) -> Self {
        self.ecc_mode = mode;
        self
    }

    /// Use ECDH-based policy binding instead of GMAC (default: false)
    ///
    /// **WARNING:** ECDSA policy binding is not yet implemented. Setting this to `true`
    /// will cause `build()` to return `NanoTdfError::Unsupported`.
    #[must_use]
    pub fn use_ecdh_binding(mut self, enabled: bool) -> Self {
        self.use_ecdh_binding = enabled;
        self
    }

    /// Set a custom rotation threshold (default: 2^23 = 8,388,608)
    ///
    /// When the IV counter reaches this value, `rotation_threshold_reached()` returns true.
    #[must_use]
    pub fn rotation_threshold(mut self, threshold: u32) -> Self {
        self.rotation_threshold = Some(threshold);
        self
    }

    /// Set the symmetric cipher (determines tag size)
    #[must_use]
    pub fn symmetric_cipher(mut self, cipher: SymmetricCipher) -> Self {
        self.symmetric_cipher = cipher;
        self
    }

    /// Build the collection by performing ECDH key derivation
    ///
    /// This performs the expensive ECDH + HKDF operation once.
    /// All subsequent `encrypt_item()` calls reuse the derived DEK.
    pub fn build(self, kas_public_key: &[u8]) -> Result<NanoTdfCollection, NanoTdfError> {
        let kas_url = self.kas_url.ok_or(NanoTdfError::MissingKasUrl)?;

        let policy_body = self.policy_body.ok_or(NanoTdfError::MissingPolicy)?;

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
        let (dek, ephemeral_public_key) = kem.derive_key_with_ephemeral(kas_public_key)?;

        // Calculate policy binding (L1L v12 format)
        let policy_binding = if self.use_ecdh_binding {
            return Err(NanoTdfError::Unsupported(
                "ECDSA policy binding not yet implemented".to_string(),
            ));
        } else {
            // GMAC binding for L1L v12: SHA-256 of policy body, take last 8 bytes
            let policy_bytes = match &policy_body {
                PolicyBody::Remote(locator) => {
                    let mut buf = Vec::new();
                    locator.write_to(&mut buf).map_err(|e| {
                        NanoTdfError::InvalidHeader(format!("Failed to serialize policy: {}", e))
                    })?;
                    buf
                }
                PolicyBody::EmbeddedPlaintext(content) => content.clone(),
                PolicyBody::EmbeddedEncrypted(content) => content.clone(),
                PolicyBody::EmbeddedEncryptedWithKeyAccess { content, .. } => content.clone(),
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
        let config = SymmetricAndPayloadConfig::new(signature_mode, self.symmetric_cipher);

        // Create ECC and binding mode
        let ecc_and_binding_mode = EccAndBindingMode::new(false, self.ecc_mode);

        // Create header
        let header = Header::new(
            kas_locator,
            ecc_and_binding_mode,
            config,
            policy,
            ephemeral_public_key.to_vec(),
        )?;

        // Determine tag size from symmetric cipher
        let tag_size = match self.symmetric_cipher {
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

        Ok(NanoTdfCollection {
            header,
            dek,
            iv_counter: AtomicU32::new(1), // Start at 1, 0 is reserved
            rotation_threshold: self
                .rotation_threshold
                .unwrap_or(DEFAULT_ROTATION_THRESHOLD),
            tag_size,
        })
    }
}

impl Default for NanoTdfCollectionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Decryptor for NanoTDF collection items
///
/// Use this when you receive a collection manifest and need to decrypt items.
/// There are two construction methods:
///
/// - `from_header_with_kas_key()` - For KAS-side or testing (has KAS private key)
/// - `from_header_with_dek()` - For client-side (after KAS rewrap returns unwrapped DEK)
pub struct NanoTdfCollectionDecryptor {
    /// The parsed header (for extracting tag size, etc.)
    header: Header,

    /// The derived or provided DEK
    dek: AesKey,

    /// Tag size from header's symmetric cipher config
    tag_size: TagSize,
}

impl NanoTdfCollectionDecryptor {
    /// Create a decryptor from header bytes and KAS private key
    ///
    /// This performs ECDH key derivation using the KAS private key and
    /// the ephemeral public key from the header. Use this for KAS-side
    /// decryption or testing scenarios.
    ///
    /// # Key Format
    ///
    /// The `kas_private_key` must be in one of these DER formats:
    /// - **SEC1 DER** - Raw elliptic curve private key (tried first)
    /// - **PKCS#8 DER** - Standard private key container (fallback)
    ///
    /// **Note:** Raw 32-byte scalar values are NOT supported. Convert using:
    /// ```ignore
    /// use p256::{SecretKey, pkcs8::EncodePrivateKey};
    /// let secret = SecretKey::from_bytes(&raw_bytes.into())?;
    /// let der = secret.to_pkcs8_der()?.as_bytes().to_vec();
    /// ```
    ///
    /// # Example
    ///
    /// ```ignore
    /// let decryptor = NanoTdfCollectionDecryptor::from_header_with_kas_key(
    ///     &header_bytes,
    ///     &kas_private_key_pkcs8_der,
    /// )?;
    /// ```
    pub fn from_header_with_kas_key(
        header_bytes: &[u8],
        kas_private_key: &[u8],
    ) -> Result<Self, NanoTdfError> {
        use opentdf_protocol::binary::BinaryRead;

        // Parse header
        let mut cursor = Cursor::new(header_bytes);
        let header = Header::read_from(&mut cursor)?;

        // Convert EccMode to EcCurve
        let curve = match header.ecc_and_binding_mode.ecc_mode {
            EccMode::Secp256r1 => EcCurve::P256,
            EccMode::Secp384r1 => EcCurve::P384,
            EccMode::Secp521r1 => EcCurve::P521,
            EccMode::Secp256k1 => EcCurve::Secp256k1,
        };

        // Create ECDH KEM for the curve
        let kem = EcdhKem::new(curve);

        // Perform ECDH + HKDF to derive decryption key
        let dek = kem.derive_key_with_private(kas_private_key, &header.ephemeral_public_key)?;

        // Determine tag size from header
        let tag_size = Self::tag_size_from_header(&header)?;

        Ok(Self {
            header,
            dek,
            tag_size,
        })
    }

    /// Create a decryptor from header bytes and an already-unwrapped DEK
    ///
    /// Use this for client-side decryption after the KAS rewrap operation
    /// has returned the unwrapped DEK.
    pub fn from_header_with_dek(header_bytes: &[u8], dek: &[u8]) -> Result<Self, NanoTdfError> {
        use opentdf_protocol::binary::BinaryRead;

        // Validate DEK length
        if dek.len() != 32 {
            return Err(NanoTdfError::InvalidDekLength(dek.len()));
        }

        // Parse header
        let mut cursor = Cursor::new(header_bytes);
        let header = Header::read_from(&mut cursor)?;

        // Create AesKey from provided DEK
        let dek = AesKey::from_slice(dek).map_err(|_| NanoTdfError::InvalidDekLength(dek.len()))?;

        // Determine tag size from header
        let tag_size = Self::tag_size_from_header(&header)?;

        Ok(Self {
            header,
            dek,
            tag_size,
        })
    }

    /// Decrypt a collection item
    pub fn decrypt_item(&self, item: &CollectionItem) -> Result<Vec<u8>, NanoTdfError> {
        // Create NanoTdfIv from item's IV counter
        let nano_iv = NanoTdfIv::from_bytes([
            ((item.iv >> 16) & 0xFF) as u8,
            ((item.iv >> 8) & 0xFF) as u8,
            (item.iv & 0xFF) as u8,
        ]);

        // Decrypt payload
        let plaintext = decrypt(&self.dek, &nano_iv, &item.ciphertext_and_tag, self.tag_size)?;

        Ok(plaintext)
    }

    /// Get reference to the header
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Get the tag size
    pub fn tag_size(&self) -> TagSize {
        self.tag_size
    }

    /// Get the authentication tag size in bytes
    pub fn tag_size_bytes(&self) -> usize {
        self.tag_size.bytes()
    }

    /// Extract tag size from header's symmetric cipher config
    fn tag_size_from_header(header: &Header) -> Result<TagSize, NanoTdfError> {
        match header.symmetric_and_payload_config.symmetric_cipher {
            SymmetricCipher::Aes256Gcm64 => {
                #[cfg(feature = "nanotdf-mbedtls")]
                {
                    Ok(TagSize::Bits64)
                }
                #[cfg(not(feature = "nanotdf-mbedtls"))]
                {
                    Err(NanoTdfError::Unsupported(
                        "64-bit GCM tags require nanotdf-mbedtls feature".to_string(),
                    ))
                }
            }
            SymmetricCipher::Aes256Gcm96 => Ok(TagSize::Bits96),
            SymmetricCipher::Aes256Gcm104 => Ok(TagSize::Bits104),
            SymmetricCipher::Aes256Gcm112 => Ok(TagSize::Bits112),
            SymmetricCipher::Aes256Gcm120 => Ok(TagSize::Bits120),
            SymmetricCipher::Aes256Gcm128 => Ok(TagSize::Bits128),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test key generation helper
    #[cfg(feature = "kas")]
    fn generate_test_keypair() -> (Vec<u8>, Vec<u8>) {
        use p256::SecretKey;
        use p256::pkcs8::EncodePrivateKey;
        use rand::rngs::OsRng;

        let secret = SecretKey::random(&mut OsRng);
        let public = secret.public_key().to_sec1_bytes().to_vec();
        let private = secret.to_pkcs8_der().unwrap().as_bytes().to_vec();
        (public, private)
    }

    #[test]
    fn test_builder_api() {
        let builder = NanoTdfCollectionBuilder::new()
            .kas_url("http://localhost:8080/kas")
            .policy_plaintext(b"test-policy".to_vec())
            .ecc_mode(EccMode::Secp256r1)
            .rotation_threshold(1000);

        assert!(builder.kas_url.is_some());
        assert!(builder.policy_body.is_some());
        assert_eq!(builder.rotation_threshold, Some(1000));
    }

    #[test]
    #[cfg(feature = "kas")]
    fn test_collection_roundtrip() {
        let (public_key, private_key) = generate_test_keypair();

        // Create collection
        let collection = NanoTdfCollectionBuilder::new()
            .kas_url("http://localhost:8080/kas")
            .policy_plaintext(b"test-policy".to_vec())
            .build(&public_key)
            .unwrap();

        // Encrypt some items
        let plaintexts = vec![
            b"Hello, World!".to_vec(),
            b"Second message".to_vec(),
            b"Third message with more data".to_vec(),
        ];

        let mut items = Vec::new();
        for plaintext in &plaintexts {
            let item = collection.encrypt_item(plaintext).unwrap();
            items.push(item);
        }

        // Verify IV uniqueness
        let ivs: Vec<u32> = items.iter().map(|i| i.iv).collect();
        assert_eq!(ivs, vec![1, 2, 3]);

        // Serialize header
        let header_bytes = collection.to_header_bytes().unwrap();

        // Create decryptor
        let decryptor =
            NanoTdfCollectionDecryptor::from_header_with_kas_key(&header_bytes, &private_key)
                .unwrap();

        // Decrypt and verify
        for (i, item) in items.iter().enumerate() {
            let decrypted = decryptor.decrypt_item(item).unwrap();
            assert_eq!(decrypted, plaintexts[i]);
        }
    }

    #[test]
    #[cfg(feature = "kas")]
    fn test_iv_counter_atomicity() {
        use std::sync::Arc;
        use std::thread;

        let (public_key, _) = generate_test_keypair();

        let collection = Arc::new(
            NanoTdfCollectionBuilder::new()
                .kas_url("http://localhost:8080/kas")
                .policy_plaintext(b"test-policy".to_vec())
                .build(&public_key)
                .unwrap(),
        );

        let num_threads = 4;
        let items_per_thread = 100;
        let mut handles = Vec::new();

        for _ in 0..num_threads {
            let coll = Arc::clone(&collection);
            handles.push(thread::spawn(move || {
                let mut ivs = Vec::new();
                for _ in 0..items_per_thread {
                    let item = coll.encrypt_item(b"test").unwrap();
                    ivs.push(item.iv);
                }
                ivs
            }));
        }

        // Collect all IVs
        let mut all_ivs = Vec::new();
        for handle in handles {
            all_ivs.extend(handle.join().unwrap());
        }

        // Verify all IVs are unique
        all_ivs.sort();
        let unique_count = all_ivs.len();
        all_ivs.dedup();
        assert_eq!(all_ivs.len(), unique_count, "All IVs should be unique");

        // Verify IVs are in valid range
        for iv in &all_ivs {
            assert!(*iv >= 1 && *iv <= MAX_IV);
        }
    }

    #[test]
    #[cfg(feature = "kas")]
    fn test_rotation_threshold() {
        let (public_key, _) = generate_test_keypair();

        // threshold=5 means: rotation_threshold_reached() returns true when current_iv >= 5
        // IV counter starts at 1 and increments after each encrypt:
        //   Initial: current_iv=1, 1>=5=false
        //   After encrypt #1: IV used=1, current_iv=2, 2>=5=false
        //   After encrypt #2: IV used=2, current_iv=3, 3>=5=false
        //   After encrypt #3: IV used=3, current_iv=4, 4>=5=false
        //   After encrypt #4: IV used=4, current_iv=5, 5>=5=TRUE
        let collection = NanoTdfCollectionBuilder::new()
            .kas_url("http://localhost:8080/kas")
            .policy_plaintext(b"test-policy".to_vec())
            .rotation_threshold(5)
            .build(&public_key)
            .unwrap();

        // Initial: counter=1
        assert!(!collection.rotation_threshold_reached());

        // Encrypt 3 items: counter goes 1->2->3->4
        for _ in 0..3 {
            collection.encrypt_item(b"test").unwrap();
            assert!(!collection.rotation_threshold_reached());
        }

        // Fourth item: counter goes 4->5, which triggers threshold (5>=5)
        collection.encrypt_item(b"test").unwrap();
        assert!(collection.rotation_threshold_reached());
    }

    #[test]
    #[cfg(feature = "kas")]
    fn test_remaining_capacity() {
        let (public_key, _) = generate_test_keypair();

        let collection = NanoTdfCollectionBuilder::new()
            .kas_url("http://localhost:8080/kas")
            .policy_plaintext(b"test-policy".to_vec())
            .build(&public_key)
            .unwrap();

        // Initial capacity
        assert_eq!(collection.remaining_capacity(), MAX_IV);

        // After encrypting one item
        collection.encrypt_item(b"test").unwrap();
        assert_eq!(collection.remaining_capacity(), MAX_IV - 1);
    }

    #[test]
    #[cfg(feature = "kas")]
    fn test_decryptor_with_dek() {
        let (public_key, private_key) = generate_test_keypair();

        // Create collection and encrypt item
        let collection = NanoTdfCollectionBuilder::new()
            .kas_url("http://localhost:8080/kas")
            .policy_plaintext(b"test-policy".to_vec())
            .build(&public_key)
            .unwrap();

        let plaintext = b"Test message for DEK decryptor";
        let item = collection.encrypt_item(plaintext).unwrap();

        // Get header and derive DEK (simulating what KAS rewrap would return)
        let header_bytes = collection.to_header_bytes().unwrap();

        // First create a decryptor with KAS key to get the DEK
        let decryptor_kas =
            NanoTdfCollectionDecryptor::from_header_with_kas_key(&header_bytes, &private_key)
                .unwrap();

        // Get DEK bytes (in real scenario this would come from KAS rewrap)
        let dek_bytes = decryptor_kas.dek.as_slice();

        // Create decryptor with provided DEK
        let decryptor_dek =
            NanoTdfCollectionDecryptor::from_header_with_dek(&header_bytes, dek_bytes).unwrap();

        // Decrypt and verify
        let decrypted = decryptor_dek.decrypt_item(&item).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_invalid_dek_length() {
        let header_bytes = vec![0x4C, 0x31, 0x4C]; // Minimal header start (will fail to parse)
        let bad_dek = vec![0u8; 16]; // Wrong length

        let result = NanoTdfCollectionDecryptor::from_header_with_dek(&header_bytes, &bad_dek);
        assert!(matches!(result, Err(NanoTdfError::InvalidDekLength(16))));
    }

    #[test]
    #[cfg(feature = "kas")]
    fn test_iv_exhaustion_error() {
        // Create collection with IV counter near MAX_IV
        // Verify IvExhausted error is returned
        let (public_key, _) = generate_test_keypair();

        let collection = NanoTdfCollectionBuilder::new()
            .kas_url("http://localhost:8080/kas")
            .policy_plaintext(b"test-policy".to_vec())
            .build(&public_key)
            .unwrap();

        // Set IV counter to MAX_IV (one encryption left)
        collection
            .iv_counter
            .store(MAX_IV, std::sync::atomic::Ordering::Relaxed);

        // Verify state before exhaustion
        assert!(!collection.is_exhausted());
        assert_eq!(collection.remaining_capacity(), 1);

        // This encryption should succeed (uses MAX_IV)
        let result = collection.encrypt_item(b"last valid item");
        assert!(result.is_ok(), "Should succeed at MAX_IV");

        // Now the collection is exhausted
        assert!(collection.is_exhausted());
        assert_eq!(collection.remaining_capacity(), 0);
        assert_eq!(collection.current_iv(), MAX_IV + 1);

        // Next encryption should fail with IvExhausted
        let result = collection.encrypt_item(b"should fail");
        assert!(
            matches!(result, Err(NanoTdfError::IvExhausted)),
            "Expected IvExhausted error, got: {:?}",
            result
        );

        // Verify multiple attempts also fail
        let result2 = collection.encrypt_item(b"still should fail");
        assert!(matches!(result2, Err(NanoTdfError::IvExhausted)));
    }
}
