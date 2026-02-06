//! High-level TDF API for simplified encryption and decryption
//!
//! This module provides a clean, fluent API for common TDF operations.

use crate::archive::{TdfArchiveBuilder, TdfArchiveMemoryBuilder, TdfError};
use crate::manifest::{IntegrityInformationExt, KeyAccessExt, TdfManifest, TdfManifestExt};
use crate::policy::Policy;
use opentdf_crypto::TdfEncryption;
use std::path::Path;

#[cfg(feature = "kas-client")]
use crate::archive::TdfArchive;

#[cfg(feature = "kas-client")]
use crate::kas::KasClient;

/// High-level TDF operations
///
/// # Examples
///
/// ```no_run
/// use opentdf::Tdf;
/// # use opentdf::Policy;
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # let policy = Policy::new("uuid".to_string(), vec![], vec![]);
///
/// // Encrypt data
/// Tdf::encrypt(b"Sensitive data")
///     .kas_url("https://kas.example.com")
///     .policy(policy)
///     .to_file("output.tdf")?;
/// # Ok(())
/// # }
/// ```
pub struct Tdf;

impl Tdf {
    /// Encrypt data to TDF format
    ///
    /// Returns a builder that allows setting KAS URL, policy, and output options.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use opentdf::{Tdf, Policy};
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let policy = Policy::new("uuid".to_string(), vec![], vec![]);
    /// let encrypted = Tdf::encrypt(b"Sensitive data")
    ///     .kas_url("https://kas.example.com")
    ///     .policy(policy)
    ///     .to_bytes()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn encrypt(data: impl Into<Vec<u8>>) -> TdfEncryptBuilder {
        TdfEncryptBuilder::new(data.into())
    }

    /// Encrypt a file to TDF format
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use opentdf::{Tdf, Policy};
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let policy = Policy::new("uuid".to_string(), vec![], vec![]);
    /// Tdf::encrypt_file("input.txt", "output.tdf")
    ///     .kas_url("https://kas.example.com")
    ///     .policy(policy)
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn encrypt_file(
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
    ) -> TdfEncryptFileBuilder {
        TdfEncryptFileBuilder::new(input.as_ref().to_path_buf(), output.as_ref().to_path_buf())
    }

    /// Decrypt TDF data using a builder pattern
    ///
    /// Returns a builder that allows setting KAS client and output options.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use opentdf::{Tdf, kas::KasClient};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let kas_client = KasClient::new("https://kas.example.com", "token")?;
    /// let tdf_data = std::fs::read("encrypted.tdf")?;
    /// let plaintext = Tdf::decrypt(tdf_data)
    ///     .kas_client(kas_client)
    ///     .to_bytes()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "kas-client")]
    pub fn decrypt(data: impl Into<Vec<u8>>) -> TdfDecryptBuilder {
        TdfDecryptBuilder::new(data.into())
    }

    /// Decrypt a TDF file using a builder pattern
    ///
    /// Returns a builder that allows setting KAS client and output options.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use opentdf::{Tdf, kas::KasClient};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let kas_client = KasClient::new("https://kas.example.com", "token")?;
    /// let plaintext = Tdf::decrypt_file("encrypted.tdf")
    ///     .kas_client(kas_client)
    ///     .to_bytes()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "kas-client")]
    pub fn decrypt_file(path: impl AsRef<Path>) -> TdfDecryptFileBuilder {
        TdfDecryptFileBuilder::new(path.as_ref().to_path_buf())
    }
}

/// Builder for encrypting data to TDF format
pub struct TdfEncryptBuilder {
    data: Vec<u8>,
    kas_url: Option<String>,
    policy: Option<Policy>,
    mime_type: Option<String>,
    segment_size: usize,
}

impl TdfEncryptBuilder {
    pub(crate) fn new(data: Vec<u8>) -> Self {
        const DEFAULT_SEGMENT_SIZE: usize = 2 * 1024 * 1024; // 2MB
        Self {
            data,
            kas_url: None,
            policy: None,
            mime_type: None,
            segment_size: DEFAULT_SEGMENT_SIZE,
        }
    }

    /// Set the KAS URL for key access
    #[must_use]
    pub fn kas_url(mut self, url: impl Into<String>) -> Self {
        self.kas_url = Some(url.into());
        self
    }

    /// Set the access control policy
    #[must_use]
    pub fn policy(mut self, policy: Policy) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Set the MIME type for the encrypted data
    #[must_use]
    pub fn mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.mime_type = Some(mime_type.into());
        self
    }

    /// Set the segment size for encryption (default: 2MB)
    #[must_use]
    pub fn segment_size(mut self, size: usize) -> Self {
        self.segment_size = size;
        self
    }

    /// Build and return encrypted TDF as bytes
    pub fn to_bytes(self) -> Result<Vec<u8>, TdfError> {
        let (manifest, segments, _encryption) = self.prepare_tdf()?;

        // Use in-memory builder
        let mut builder = TdfArchiveMemoryBuilder::new();
        builder.add_entry_with_segments(&manifest, &segments, 0)?;
        builder.finish()
    }

    /// Build and write encrypted TDF to file
    pub fn to_file(self, path: impl AsRef<Path>) -> Result<(), TdfError> {
        let (manifest, segments, _encryption) = self.prepare_tdf()?;

        // Use file-based builder
        let mut builder = TdfArchiveBuilder::new(path.as_ref())?;
        builder.add_entry_with_segments(&manifest, &segments, 0)?;
        builder.finish()?;
        Ok(())
    }

    /// Prepare TDF components (manifest, segments, encryption)
    ///
    /// This shared method is used by both to_bytes() and to_file()
    fn prepare_tdf(self) -> Result<(TdfManifest, Vec<Vec<u8>>, TdfEncryption), TdfError> {
        // Validate required fields
        let kas_url = self
            .kas_url
            .ok_or_else(|| TdfError::MissingRequiredField { field: "kas_url" })?;

        // Create encryption
        let tdf_encryption = TdfEncryption::new().map_err(|e| TdfError::CryptoError {
            algorithm: "AES-256-GCM".to_string(),
            reason: "Encryption setup failed".to_string(),
            source: Some(Box::new(e)),
        })?;

        // Encrypt with segments
        let segmented = tdf_encryption
            .encrypt_with_segments(&self.data, self.segment_size)
            .map_err(|e| TdfError::CryptoError {
                algorithm: "AES-256-GCM".to_string(),
                reason: "Encryption failed".to_string(),
                source: Some(Box::new(e)),
            })?;

        // Build manifest
        let mut manifest = TdfManifest::new("0.payload".to_string(), kas_url);
        manifest.encryption_information.method.algorithm = "AES-256-GCM".to_string();
        manifest.encryption_information.method.iv = String::new(); // Segments have their own IVs

        if let Some(mime_type) = self.mime_type {
            manifest.payload.mime_type = Some(mime_type);
        }

        // Set policy if provided
        if let Some(policy) = self.policy {
            manifest
                .set_policy(&policy)
                .map_err(|e| TdfError::PolicyValidationFailed {
                    policy_id: policy.uuid.clone(),
                    errors: vec![e.to_string()],
                })?;

            // Generate policy binding
            let policy_json = policy
                .to_json()
                .map_err(|e| TdfError::PolicyValidationFailed {
                    policy_id: policy.uuid.clone(),
                    errors: vec![format!("Policy serialization failed: {}", e)],
                })?;

            manifest.encryption_information.key_access[0]
                .generate_policy_binding_raw(&policy_json, tdf_encryption.payload_key())
                .map_err(|e| TdfError::PolicyValidationFailed {
                    policy_id: policy.uuid.clone(),
                    errors: vec![format!("Policy binding generation failed: {}", e)],
                })?;
        }

        // Add segment information
        use crate::manifest::Segment;
        for seg_info in &segmented.segment_info {
            manifest
                .encryption_information
                .integrity_information
                .segments
                .push(Segment {
                    hash: seg_info.hash.clone(),
                    segment_size: Some(seg_info.plaintext_size),
                    encrypted_segment_size: Some(seg_info.encrypted_size),
                });
        }

        // Set segment defaults
        if let Some(first_seg) = segmented.segment_info.first() {
            manifest
                .encryption_information
                .integrity_information
                .segment_size_default = first_seg.plaintext_size;
            manifest
                .encryption_information
                .integrity_information
                .encrypted_segment_size_default = first_seg.encrypted_size;
        }

        // Generate root signature
        manifest
            .encryption_information
            .integrity_information
            .generate_root_signature(&segmented.gmac_tags, tdf_encryption.payload_key())
            .map_err(|e| TdfError::CryptoError {
                algorithm: "HMAC-SHA256".to_string(),
                reason: format!("Root signature generation failed: {}", e),
                source: None,
            })?;

        Ok((manifest, segmented.segments, tdf_encryption))
    }
}

/// Builder for encrypting files to TDF format
pub struct TdfEncryptFileBuilder {
    input_path: std::path::PathBuf,
    output_path: std::path::PathBuf,
    kas_url: Option<String>,
    policy: Option<Policy>,
    mime_type: Option<String>,
    segment_size: usize,
}

impl TdfEncryptFileBuilder {
    pub(crate) fn new(input_path: std::path::PathBuf, output_path: std::path::PathBuf) -> Self {
        const DEFAULT_SEGMENT_SIZE: usize = 2 * 1024 * 1024; // 2MB
        Self {
            input_path,
            output_path,
            kas_url: None,
            policy: None,
            mime_type: None,
            segment_size: DEFAULT_SEGMENT_SIZE,
        }
    }

    /// Set the KAS URL for key access
    #[must_use]
    pub fn kas_url(mut self, url: impl Into<String>) -> Self {
        self.kas_url = Some(url.into());
        self
    }

    /// Set the access control policy
    #[must_use]
    pub fn policy(mut self, policy: Policy) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Set the MIME type for the encrypted file
    #[must_use]
    pub fn mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.mime_type = Some(mime_type.into());
        self
    }

    /// Set the segment size for encryption (default: 2MB)
    #[must_use]
    pub fn segment_size(mut self, size: usize) -> Self {
        self.segment_size = size;
        self
    }

    /// Build the encrypted TDF file
    pub fn build(self) -> Result<(), TdfError> {
        let data = std::fs::read(&self.input_path)?;

        let mut builder = TdfEncryptBuilder::new(data);

        if let Some(kas_url) = self.kas_url {
            builder = builder.kas_url(kas_url);
        }

        if let Some(policy) = self.policy {
            builder = builder.policy(policy);
        }

        if let Some(mime_type) = self.mime_type {
            builder = builder.mime_type(mime_type);
        }

        builder = builder.segment_size(self.segment_size);

        builder.to_file(self.output_path)
    }
}

/// Builder for decrypting TDF data
///
/// Provides a fluent API for TDF decryption with optional KAS integration.
#[cfg(feature = "kas-client")]
pub struct TdfDecryptBuilder {
    data: Vec<u8>,
    kas_client: Option<KasClient>,
}

#[cfg(feature = "kas-client")]
impl TdfDecryptBuilder {
    pub(crate) fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            kas_client: None,
        }
    }

    /// Set the KAS client for key unwrapping
    #[must_use]
    pub fn kas_client(mut self, client: KasClient) -> Self {
        self.kas_client = Some(client);
        self
    }

    /// Decrypt and return plaintext as bytes
    pub async fn to_bytes(self) -> Result<Vec<u8>, TdfError> {
        let kas_client = self
            .kas_client
            .ok_or_else(|| TdfError::MissingRequiredField {
                field: "kas_client",
            })?;

        let cursor = std::io::Cursor::new(self.data);
        let mut archive = TdfArchive::new(cursor)?;
        let entry = archive.by_index()?;
        entry.decrypt_with_kas(&kas_client).await
    }

    /// Decrypt and write plaintext to file
    pub async fn to_file(self, path: impl AsRef<Path>) -> Result<(), TdfError> {
        let plaintext = self.to_bytes().await?;
        std::fs::write(path, plaintext)?;
        Ok(())
    }
}

/// Builder for decrypting TDF files
///
/// Provides a fluent API for file-based TDF decryption with optional KAS integration.
#[cfg(feature = "kas-client")]
pub struct TdfDecryptFileBuilder {
    input_path: std::path::PathBuf,
    kas_client: Option<KasClient>,
}

#[cfg(feature = "kas-client")]
impl TdfDecryptFileBuilder {
    pub(crate) fn new(input_path: std::path::PathBuf) -> Self {
        Self {
            input_path,
            kas_client: None,
        }
    }

    /// Set the KAS client for key unwrapping
    #[must_use]
    pub fn kas_client(mut self, client: KasClient) -> Self {
        self.kas_client = Some(client);
        self
    }

    /// Decrypt and return plaintext as bytes
    pub async fn to_bytes(self) -> Result<Vec<u8>, TdfError> {
        let kas_client = self
            .kas_client
            .ok_or_else(|| TdfError::MissingRequiredField {
                field: "kas_client",
            })?;

        TdfArchive::open_and_decrypt(self.input_path, &kas_client).await
    }

    /// Decrypt and write plaintext to file
    pub async fn to_file(self, output_path: impl AsRef<Path>) -> Result<(), TdfError> {
        let plaintext = self.to_bytes().await?;
        std::fs::write(output_path, plaintext)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_encrypt_to_bytes() -> Result<(), Box<dyn std::error::Error>> {
        let data = b"Hello, TDF!";

        let encrypted = Tdf::encrypt(data)
            .kas_url("https://kas.example.com")
            .to_bytes()?;

        assert!(!encrypted.is_empty());
        Ok(())
    }

    #[test]
    fn test_encrypt_to_file() -> Result<(), Box<dyn std::error::Error>> {
        let temp_file = NamedTempFile::new()?;
        let data = b"Hello, TDF!";

        Tdf::encrypt(data)
            .kas_url("https://kas.example.com")
            .to_file(temp_file.path())?;

        assert!(temp_file.path().exists());
        Ok(())
    }

    #[test]
    fn test_encrypt_file() -> Result<(), Box<dyn std::error::Error>> {
        let input_file = NamedTempFile::new()?;
        let output_file = NamedTempFile::new()?;

        std::fs::write(input_file.path(), b"File contents")?;

        Tdf::encrypt_file(input_file.path(), output_file.path())
            .kas_url("https://kas.example.com")
            .build()?;

        assert!(output_file.path().exists());
        Ok(())
    }

    #[test]
    fn test_missing_kas_url_error() {
        let data = b"Hello, TDF!";

        let result = Tdf::encrypt(data).to_bytes();

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.error_code(), "OPENTDF_E_FIELD_REQUIRED");
        assert!(err.to_string().contains("kas_url"));
    }

    #[test]
    fn test_with_policy() -> Result<(), Box<dyn std::error::Error>> {
        let policy = Policy::new(
            "test-uuid".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );

        let temp_file = NamedTempFile::new()?;

        Tdf::encrypt(b"Test data")
            .kas_url("https://kas.example.com")
            .policy(policy)
            .mime_type("text/plain")
            .to_file(temp_file.path())?;

        assert!(temp_file.path().exists());
        Ok(())
    }
}
