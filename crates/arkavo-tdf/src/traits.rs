//! Core traits for TDF operations with streaming support.
//!
//! All traits are designed streaming-first to handle GB-scale data without OOM.
//! Convenience methods for small data (< 64MB) delegate to streaming internally.

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::error::{TdfError, TransportError};
use crate::types::{Policy, TdfManifest};

/// Streaming TDF encryption - handles GB-scale data without OOM.
///
/// Implementations should use chunked encryption for large payloads.
#[async_trait]
pub trait TdfEncryptor: Send + Sync {
    /// Encrypt small data (convenience method for payloads < 64MB).
    ///
    /// For larger data, use `encrypt_stream` to avoid memory issues.
    async fn encrypt(&self, plaintext: &[u8], policy: &Policy) -> Result<TdfManifest, TdfError>;

    /// Streaming encryption for large data (models, videos, etc.).
    ///
    /// Reads from the async reader and produces a TDF manifest with
    /// the encrypted payload. The manifest's `payload.value` contains
    /// the base64-encoded ciphertext.
    async fn encrypt_stream<R>(&self, reader: R, policy: &Policy) -> Result<TdfManifest, TdfError>
    where
        R: AsyncRead + Send + Unpin;
}

/// Streaming TDF decryption - handles GB-scale data without OOM.
#[async_trait]
pub trait TdfDecryptor: Send + Sync {
    /// Decrypt small TDF (convenience method for payloads < 64MB).
    ///
    /// For larger data, use `decrypt_stream` to avoid memory issues.
    async fn decrypt(&self, manifest: &TdfManifest) -> Result<Vec<u8>, TdfError>;

    /// Streaming decryption - writes plaintext to async writer.
    ///
    /// Returns the number of bytes written.
    async fn decrypt_stream<W>(&self, manifest: &TdfManifest, writer: W) -> Result<u64, TdfError>
    where
        W: AsyncWrite + Send + Unpin;
}

/// Key Access Service (KAS) client for key operations.
///
/// KAS handles key wrapping/unwrapping with policy binding.
///
/// Note: This trait uses `?Send` because the underlying opentdf-rs KasClient
/// contains non-Send types (RSA keys with raw pointers). Use with care in
/// multi-threaded contexts.
#[async_trait(?Send)]
pub trait KasClient {
    /// Rewrap a wrapped key for the current entity.
    ///
    /// The KAS verifies the entity's attributes against the policy
    /// before releasing the unwrapped key material.
    async fn rewrap(&self, wrapped_key: &str, policy: &str) -> Result<Vec<u8>, TdfError>;

    /// Check if KAS is available and healthy.
    async fn health_check(&self) -> Result<bool, TdfError>;
}

/// Blob transport - DECOUPLED from encryption.
///
/// Handles raw byte transport (streaming). Encryption is composed
/// at the application layer by combining `TdfEncryptor` + `BlobTransport`.
///
/// This separation allows:
/// - Testing transport without encryption (Phase 2)
/// - Different transport backends (Iroh, S3, etc.)
/// - Independent scaling of encryption and transport
#[async_trait]
pub trait BlobTransport: Send + Sync {
    /// Stage bytes from reader (streaming) - for large blobs.
    ///
    /// Returns a ticket that can be used to fetch the blob later.
    async fn stage_stream<R>(&self, reader: R) -> Result<String, TransportError>
    where
        R: AsyncRead + Send + Unpin;

    /// Fetch bytes to writer (streaming) - for large blobs.
    ///
    /// Returns the number of bytes fetched.
    async fn fetch_stream<W>(&self, ticket: &str, writer: W) -> Result<u64, TransportError>
    where
        W: AsyncWrite + Send + Unpin;

    /// Stage small data (convenience method for payloads < 64MB).
    async fn stage(&self, data: &[u8]) -> Result<String, TransportError>;

    /// Fetch small data (convenience method for payloads < 64MB).
    async fn fetch(&self, ticket: &str) -> Result<Vec<u8>, TransportError>;

    /// Check if the transport node is available.
    async fn health_check(&self) -> Result<bool, TransportError>;
}

/// Combined TDF service providing both encryption and decryption.
///
/// Implementations can use this marker trait to indicate they
/// provide a complete TDF solution.
pub trait TdfService: TdfEncryptor + TdfDecryptor + Send + Sync {}

impl<T: TdfEncryptor + TdfDecryptor + Send + Sync> TdfService for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct DummyEncryptor;

    #[async_trait]
    impl TdfEncryptor for DummyEncryptor {
        async fn encrypt(
            &self,
            _plaintext: &[u8],
            _policy: &Policy,
        ) -> Result<TdfManifest, TdfError> {
            Err(TdfError::Encryption("dummy".to_string()))
        }

        async fn encrypt_stream<R>(
            &self,
            _reader: R,
            _policy: &Policy,
        ) -> Result<TdfManifest, TdfError>
        where
            R: AsyncRead + Send + Unpin,
        {
            Err(TdfError::Encryption("dummy".to_string()))
        }
    }

    #[async_trait]
    impl TdfDecryptor for DummyEncryptor {
        async fn decrypt(&self, _manifest: &TdfManifest) -> Result<Vec<u8>, TdfError> {
            Err(TdfError::Decryption("dummy".to_string()))
        }

        async fn decrypt_stream<W>(
            &self,
            _manifest: &TdfManifest,
            _writer: W,
        ) -> Result<u64, TdfError>
        where
            W: AsyncWrite + Send + Unpin,
        {
            Err(TdfError::Decryption("dummy".to_string()))
        }
    }

    #[test]
    fn tdf_service_auto_impl() {
        fn assert_tdf_service<T: TdfService>() {}
        assert_tdf_service::<DummyEncryptor>();
    }

    #[tokio::test]
    async fn dummy_encryptor_returns_error() {
        let encryptor = DummyEncryptor;
        let policy = Policy::default();

        let result = encryptor.encrypt(b"test", &policy).await;
        assert!(result.is_err());

        let cursor = Cursor::new(b"test".to_vec());
        let result = encryptor.encrypt_stream(cursor, &policy).await;
        assert!(result.is_err());
    }
}
