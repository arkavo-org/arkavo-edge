//! Mock implementations for testing TDF operations.
//!
//! Enable with the `testing` feature flag.

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::{TdfError, TransportError};
use crate::traits::{BlobTransport, KasClient, TdfDecryptor, TdfEncryptor};
use crate::types::{
    EncryptionInformation, EncryptionMethod, InlinePayload, KeyAccessObject, Policy, PolicyBinding,
    TdfManifest,
};

/// Mock TDF encryptor/decryptor for testing.
///
/// Performs simple XOR "encryption" - NOT secure, only for testing.
#[derive(Debug, Clone)]
pub struct MockTdfService {
    key: u8,
    kas_url: String,
}

impl MockTdfService {
    /// Create a new mock TDF service with the given XOR key.
    #[must_use]
    pub fn new(key: u8) -> Self {
        Self {
            key,
            kas_url: "https://mock-kas.arkavo.net".to_string(),
        }
    }

    /// Create with a custom KAS URL.
    #[must_use]
    pub fn with_kas_url(mut self, url: &str) -> Self {
        self.kas_url = url.to_string();
        self
    }

    fn xor_bytes(&self, data: &[u8]) -> Vec<u8> {
        data.iter().map(|b| b ^ self.key).collect()
    }

    fn create_manifest(&self, ciphertext: &[u8], policy: &Policy) -> TdfManifest {
        let policy_json = serde_json::to_string(policy).unwrap_or_default();
        let policy_b64 = BASE64.encode(policy_json.as_bytes());

        TdfManifest {
            encryption_information: EncryptionInformation {
                key_type: "split".to_string(),
                key_access: vec![KeyAccessObject {
                    access_type: "wrapped".to_string(),
                    url: self.kas_url.clone(),
                    protocol: "kas".to_string(),
                    wrapped_key: BASE64.encode([self.key]),
                    policy_binding: PolicyBinding {
                        alg: "HS256".to_string(),
                        hash: "mock_hash".to_string(),
                    },
                }],
                method: EncryptionMethod {
                    algorithm: "XOR-MOCK".to_string(),
                    iv: "mock_iv".to_string(),
                    is_streamable: true,
                },
                policy: policy_b64,
            },
            payload: InlinePayload {
                payload_type: "inline".to_string(),
                mime_type: "application/octet-stream".to_string(),
                protocol: "base64".to_string(),
                value: BASE64.encode(ciphertext),
            },
            version: "3.0.0".to_string(),
        }
    }
}

impl Default for MockTdfService {
    fn default() -> Self {
        Self::new(0x42)
    }
}

#[async_trait]
impl TdfEncryptor for MockTdfService {
    async fn encrypt(&self, plaintext: &[u8], policy: &Policy) -> Result<TdfManifest, TdfError> {
        let ciphertext = self.xor_bytes(plaintext);
        Ok(self.create_manifest(&ciphertext, policy))
    }

    async fn encrypt_stream<R>(
        &self,
        mut reader: R,
        policy: &Policy,
    ) -> Result<TdfManifest, TdfError>
    where
        R: AsyncRead + Send + Unpin,
    {
        let mut buffer = Vec::new();
        reader.read_to_end(&mut buffer).await?;
        self.encrypt(&buffer, policy).await
    }
}

#[async_trait]
impl TdfDecryptor for MockTdfService {
    async fn decrypt(&self, manifest: &TdfManifest) -> Result<Vec<u8>, TdfError> {
        let ciphertext = BASE64
            .decode(&manifest.payload.value)
            .map_err(|e| TdfError::Base64(e.to_string()))?;
        Ok(self.xor_bytes(&ciphertext))
    }

    async fn decrypt_stream<W>(
        &self,
        manifest: &TdfManifest,
        mut writer: W,
    ) -> Result<u64, TdfError>
    where
        W: AsyncWrite + Send + Unpin,
    {
        let plaintext = self.decrypt(manifest).await?;
        writer.write_all(&plaintext).await?;
        Ok(plaintext.len() as u64)
    }
}

/// Mock KAS client for testing.
#[derive(Debug, Clone, Default)]
pub struct MockKasClient {
    responses: Arc<Mutex<HashMap<String, Result<Vec<u8>, String>>>>,
    health: Arc<Mutex<bool>>,
}

impl MockKasClient {
    /// Create a new mock KAS client.
    #[must_use]
    pub fn new() -> Self {
        Self {
            responses: Arc::new(Mutex::new(HashMap::new())),
            health: Arc::new(Mutex::new(true)),
        }
    }

    /// Configure a successful rewrap response.
    pub fn with_rewrap_response(self, policy: &str, key: Vec<u8>) -> Self {
        self.responses
            .lock()
            .unwrap()
            .insert(policy.to_string(), Ok(key));
        self
    }

    /// Configure an error rewrap response.
    pub fn with_rewrap_error(self, policy: &str, error: &str) -> Self {
        self.responses
            .lock()
            .unwrap()
            .insert(policy.to_string(), Err(error.to_string()));
        self
    }

    /// Set health check result.
    pub fn set_healthy(&self, healthy: bool) {
        *self.health.lock().unwrap() = healthy;
    }
}

#[async_trait(?Send)]
impl KasClient for MockKasClient {
    async fn rewrap(&self, _wrapped_key: &str, policy: &str) -> Result<Vec<u8>, TdfError> {
        let responses = self.responses.lock().unwrap();
        match responses.get(policy) {
            Some(Ok(key)) => Ok(key.clone()),
            Some(Err(e)) => Err(TdfError::Kas(e.clone())),
            None => Err(TdfError::Kas("No mock response configured".to_string())),
        }
    }

    async fn health_check(&self) -> Result<bool, TdfError> {
        Ok(*self.health.lock().unwrap())
    }
}

/// Mock blob transport for testing.
#[derive(Debug, Clone, Default)]
pub struct MockBlobTransport {
    blobs: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    next_ticket: Arc<Mutex<u64>>,
    healthy: Arc<Mutex<bool>>,
}

impl MockBlobTransport {
    /// Create a new mock blob transport.
    #[must_use]
    pub fn new() -> Self {
        Self {
            blobs: Arc::new(Mutex::new(HashMap::new())),
            next_ticket: Arc::new(Mutex::new(1)),
            healthy: Arc::new(Mutex::new(true)),
        }
    }

    /// Pre-populate a blob for fetching.
    pub fn with_blob(self, ticket: &str, data: Vec<u8>) -> Self {
        self.blobs.lock().unwrap().insert(ticket.to_string(), data);
        self
    }

    /// Set health check result.
    pub fn set_healthy(&self, healthy: bool) {
        *self.healthy.lock().unwrap() = healthy;
    }

    /// Get all stored blobs (for assertions).
    pub fn get_blobs(&self) -> HashMap<String, Vec<u8>> {
        self.blobs.lock().unwrap().clone()
    }
}

#[async_trait]
impl BlobTransport for MockBlobTransport {
    async fn stage_stream<R>(&self, mut reader: R) -> Result<String, TransportError>
    where
        R: AsyncRead + Send + Unpin,
    {
        let mut buffer = Vec::new();
        reader
            .read_to_end(&mut buffer)
            .await
            .map_err(TransportError::Io)?;
        self.stage(&buffer).await
    }

    async fn fetch_stream<W>(&self, ticket: &str, mut writer: W) -> Result<u64, TransportError>
    where
        W: AsyncWrite + Send + Unpin,
    {
        let data = self.fetch(ticket).await?;
        writer.write_all(&data).await.map_err(TransportError::Io)?;
        Ok(data.len() as u64)
    }

    async fn stage(&self, data: &[u8]) -> Result<String, TransportError> {
        let mut ticket_id = self.next_ticket.lock().unwrap();
        let ticket = format!("mock-ticket-{ticket_id}");
        *ticket_id += 1;

        self.blobs
            .lock()
            .unwrap()
            .insert(ticket.clone(), data.to_vec());
        Ok(ticket)
    }

    async fn fetch(&self, ticket: &str) -> Result<Vec<u8>, TransportError> {
        self.blobs
            .lock()
            .unwrap()
            .get(ticket)
            .cloned()
            .ok_or_else(|| TransportError::Fetch(format!("Ticket not found: {ticket}")))
    }

    async fn health_check(&self) -> Result<bool, TransportError> {
        Ok(*self.healthy.lock().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PolicyBuilder;
    use std::io::Cursor;

    #[tokio::test]
    async fn mock_tdf_encrypt_decrypt_roundtrip() {
        let service = MockTdfService::default();
        let policy = PolicyBuilder::new()
            .attribute("https://test.com/attr/role", &["user"])
            .build()
            .unwrap();

        let plaintext = b"Hello, World!";
        let manifest = service.encrypt(plaintext, &policy).await.unwrap();

        assert_eq!(manifest.version, "3.0.0");
        assert_eq!(manifest.encryption_information.method.algorithm, "XOR-MOCK");

        let decrypted = service.decrypt(&manifest).await.unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[tokio::test]
    async fn mock_tdf_stream_encrypt_decrypt() {
        let service = MockTdfService::new(0xAB);
        let policy = PolicyBuilder::new()
            .attribute("https://test.com/attr/test", &["value"])
            .build()
            .unwrap();

        let plaintext = b"Streaming test data";
        let reader = Cursor::new(plaintext.to_vec());

        let manifest = service.encrypt_stream(reader, &policy).await.unwrap();

        let mut output = Vec::new();
        let bytes_written = service
            .decrypt_stream(&manifest, &mut output)
            .await
            .unwrap();

        assert_eq!(bytes_written, plaintext.len() as u64);
        assert_eq!(output, plaintext);
    }

    #[tokio::test]
    async fn mock_kas_client() {
        let kas = MockKasClient::new()
            .with_rewrap_response("policy1", vec![1, 2, 3])
            .with_rewrap_error("policy2", "Access denied");

        let result = kas.rewrap("wrapped", "policy1").await.unwrap();
        assert_eq!(result, vec![1, 2, 3]);

        let result = kas.rewrap("wrapped", "policy2").await;
        assert!(result.is_err());

        assert!(kas.health_check().await.unwrap());

        kas.set_healthy(false);
        assert!(!kas.health_check().await.unwrap());
    }

    #[tokio::test]
    async fn mock_blob_transport_stage_fetch() {
        let transport = MockBlobTransport::new();

        let data = b"test blob data";
        let ticket = transport.stage(data).await.unwrap();
        assert!(ticket.starts_with("mock-ticket-"));

        let fetched = transport.fetch(&ticket).await.unwrap();
        assert_eq!(fetched, data);

        let result = transport.fetch("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn mock_blob_transport_streaming() {
        let transport = MockBlobTransport::new();

        let data = b"streaming blob test";
        let reader = Cursor::new(data.to_vec());

        let ticket = transport.stage_stream(reader).await.unwrap();

        let mut output = Vec::new();
        let bytes = transport.fetch_stream(&ticket, &mut output).await.unwrap();

        assert_eq!(bytes, data.len() as u64);
        assert_eq!(output, data);
    }

    #[tokio::test]
    async fn mock_blob_transport_prepopulated() {
        let transport = MockBlobTransport::new().with_blob("preexisting-ticket", vec![10, 20, 30]);

        let fetched = transport.fetch("preexisting-ticket").await.unwrap();
        assert_eq!(fetched, vec![10, 20, 30]);
    }

    #[tokio::test]
    async fn mock_blob_transport_health() {
        let transport = MockBlobTransport::new();
        assert!(transport.health_check().await.unwrap());

        transport.set_healthy(false);
        assert!(!transport.health_check().await.unwrap());
    }

    #[test]
    fn mock_blob_transport_get_blobs() {
        let transport = MockBlobTransport::new()
            .with_blob("ticket1", vec![1])
            .with_blob("ticket2", vec![2]);

        let blobs = transport.get_blobs();
        assert_eq!(blobs.len(), 2);
        assert_eq!(blobs.get("ticket1"), Some(&vec![1]));
    }
}
