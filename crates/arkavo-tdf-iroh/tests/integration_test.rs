//! Integration tests demonstrating arkavo-tdf + arkavo-tdf-iroh composition.
//!
//! These tests show how to compose the BlobTransport trait with TDF operations.

use arkavo_tdf::testing::{MockBlobTransport, MockTdfService};
use arkavo_tdf::{BlobTransport, TdfDecryptor, TdfEncryptor};
use arkavo_tdf_iroh::IrohTransport;
use std::io::Cursor;

/// Verify IrohTransport implements BlobTransport trait correctly.
#[tokio::test]
async fn iroh_implements_blob_transport() {
    let transport = IrohTransport::new().await.unwrap();

    // Verify trait methods work
    let data = b"Trait implementation test";
    let ticket = BlobTransport::stage(&transport, data).await.unwrap();
    let fetched = BlobTransport::fetch(&transport, &ticket).await.unwrap();

    assert_eq!(fetched, data);

    transport.shutdown().await.unwrap();
}

/// Test composed workflow: encrypt -> stage -> fetch -> decrypt.
/// Uses mock encryptor + real Iroh transport.
#[tokio::test]
async fn composed_encrypt_stage_fetch_decrypt() {
    let tdf_service = MockTdfService::default();
    let transport = IrohTransport::new().await.unwrap();

    // Original data
    let plaintext = b"Secret message for TDF workflow";
    let policy = arkavo_tdf::Policy::default();

    // Encrypt the data
    let manifest = tdf_service.encrypt(plaintext, &policy).await.unwrap();

    // Get encrypted payload value (base64 encoded in manifest.payload.value)
    let encrypted_value = manifest.payload.value.as_bytes().to_vec();

    // Stage encrypted payload via Iroh
    let ticket = transport.stage(&encrypted_value).await.unwrap();

    // Fetch encrypted payload from Iroh
    let fetched_encrypted = transport.fetch(&ticket).await.unwrap();
    assert_eq!(fetched_encrypted, encrypted_value);

    // Decrypt using the service (it will decode from base64)
    let decrypted = tdf_service.decrypt(&manifest).await.unwrap();

    // Verify we got the original plaintext back
    assert_eq!(decrypted, plaintext);

    transport.shutdown().await.unwrap();
}

/// Test interoperability between mock and real transports.
#[tokio::test]
async fn mock_and_real_transport_same_interface() {
    let mock_transport = MockBlobTransport::new();
    let real_transport = IrohTransport::new().await.unwrap();

    let data = b"Same data, different transports";

    // Both should work the same way via trait
    let mock_ticket = mock_transport.stage(data).await.unwrap();
    let real_ticket = real_transport.stage(data).await.unwrap();

    // Tickets are different (different storage backends)
    assert_ne!(mock_ticket, real_ticket);

    // But data should be retrievable from each
    let mock_data = mock_transport.fetch(&mock_ticket).await.unwrap();
    let real_data = real_transport.fetch(&real_ticket).await.unwrap();

    assert_eq!(mock_data, data);
    assert_eq!(real_data, data);

    real_transport.shutdown().await.unwrap();
}

/// Test streaming workflow with mock encryption.
#[tokio::test]
async fn streaming_encrypted_workflow() {
    let tdf_service = MockTdfService::default();
    let transport = IrohTransport::new().await.unwrap();

    // Larger data for streaming
    let plaintext: Vec<u8> = (0..10000).map(|i| (i % 256) as u8).collect();
    let policy = arkavo_tdf::Policy::default();

    // Encrypt via stream
    let reader = Cursor::new(plaintext.clone());
    let manifest = tdf_service.encrypt_stream(reader, &policy).await.unwrap();

    // Stage the base64 encoded payload value
    let encrypted_value = manifest.payload.value.as_bytes().to_vec();

    // Use streaming stage
    let reader = Cursor::new(encrypted_value.clone());
    let ticket = transport.stage_stream(reader).await.unwrap();

    // Use streaming fetch
    let mut output = Vec::new();
    let bytes = transport.fetch_stream(&ticket, &mut output).await.unwrap();

    assert_eq!(bytes, encrypted_value.len() as u64);
    assert_eq!(output, encrypted_value);

    transport.shutdown().await.unwrap();
}

/// Test multiple transports sharing an Iroh node.
#[tokio::test]
async fn shared_node_multiple_transports() {
    use arkavo_tdf_iroh::IrohNode;
    use std::sync::Arc;

    // Create shared node
    let node = Arc::new(IrohNode::with_defaults());
    node.start().await.unwrap();

    // Create multiple transports from same node
    let transport1 = IrohTransport::from_node(node.clone());
    let transport2 = IrohTransport::from_node(node.clone());

    // Stage from transport1
    let data = b"Shared node data";
    let ticket = transport1.stage(data).await.unwrap();

    // Fetch from transport2 (same node, so same store)
    let fetched = transport2.fetch(&ticket).await.unwrap();
    assert_eq!(fetched, data);

    // Shutdown via node
    node.stop().await.unwrap();
}

/// Test health check consistency.
#[tokio::test]
async fn health_check_lifecycle() {
    let transport = IrohTransport::new().await.unwrap();

    // Should be healthy after creation
    assert!(transport.health_check().await.unwrap());

    // Do some work
    let _ = transport.stage(b"test").await.unwrap();

    // Still healthy
    assert!(transport.health_check().await.unwrap());

    // Shutdown
    transport.shutdown().await.unwrap();

    // Not healthy after shutdown
    assert!(!transport.health_check().await.unwrap());
}

/// Test end-to-end encryption workflow with decryption.
#[tokio::test]
async fn full_encrypt_transport_decrypt_workflow() {
    let tdf_service = MockTdfService::new(0xAB); // Use specific key
    let transport = IrohTransport::new().await.unwrap();

    let plaintext = b"Confidential: Project roadmap for Q4";
    let policy = arkavo_tdf::PolicyBuilder::new()
        .attribute("https://arkavo.net/attr/classification", &["confidential"])
        .build()
        .unwrap();

    // Encrypt
    let manifest = tdf_service.encrypt(plaintext, &policy).await.unwrap();

    // Transport the entire manifest as JSON
    let manifest_json = serde_json::to_vec(&manifest).unwrap();
    let ticket = transport.stage(&manifest_json).await.unwrap();

    // Later: fetch and decrypt
    let fetched_json = transport.fetch(&ticket).await.unwrap();
    let fetched_manifest: arkavo_tdf::TdfManifest =
        serde_json::from_slice(&fetched_json).unwrap();

    let decrypted = tdf_service.decrypt(&fetched_manifest).await.unwrap();

    assert_eq!(decrypted, plaintext);

    transport.shutdown().await.unwrap();
}
